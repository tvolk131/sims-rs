//! Classic Sims 1 cheat console (`Ctrl+Shift+C`) top-left overlay and pure-Rust
//! command parser.
//!
//! Renders a translucent/framed single-line console bar in the top-left corner
//! using `cTSWinTextEdit2` dark-navy styling (`rgb565(0x000a)`), a 500 ms
//! blinking caret ([`CARET_BLINK_US`]), up/down command history, and a concise
//! status/error feedback line.
//!
//! Emits typed [`CheatCommand`] values for `sims-game` / `sims-vm` to handle
//! cleanly without coupling.

use crate::Result;
use crate::art::Art;
use crate::dialog::{BACKGROUND, CARET_BLINK_US};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::hud::rgb565;
use crate::neighborhood::{self, art_id::POPUP_TILER};
use sims_content::ui_graphics::UiGraphics;
use std::fmt;

/// `UIGraphics` resource IDs used by [`CheatConsole`]:
/// `HotspotPopupTiler.TGA` (`5107`) and `PopupInfoTiles.bmp` (`3008`).
pub const RESOURCES: [i32; 2] = [POPUP_TILER, BACKGROUND];

/// Maximum characters accepted in the single-line cheat console input.
pub const MAX_INPUT_LEN: usize = 256;
/// Maximum command history entries retained.
pub const MAX_HISTORY: usize = 64;
/// Maximum commands emitted from a single `;`-separated chain (guards against
/// unbounded `;!;!;...` repetition).
pub const MAX_CHAIN_COMMANDS: usize = 256;
/// Simoleons awarded per `rosebud` / `klapaucius` invocation.
pub const ROSEBUD_AMOUNT: i32 = 1_000;

/// Loads the native 1.91 9-slice frames and bitmap fonts used by
/// [`CheatConsole`].
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

const ENTRY_FILL: [u8; 3] = rgb565(0x000a);
const ENTRY_BORDER: [u8; 3] = [88, 120, 204];
const ENTRY_FOCUS_BORDER: [u8; 3] = [0, 255, 255];
const BAR_FALLBACK_BG: [u8; 3] = [14, 22, 76];
const TEXT_INPUT: [u8; 3] = [255, 255, 255];
const TEXT_PROMPT: [u8; 3] = [0, 255, 255];
const TEXT_INFO: [u8; 3] = [195, 205, 205];
const TEXT_OK: [u8; 3] = [120, 238, 160];
const TEXT_ERROR: [u8; 3] = [255, 118, 118];

/// Typed classic Sims 1 cheat command emitted by [`parse_cheat_line`] and
/// [`CheatConsole::submit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheatCommand {
    /// `rosebud` or `klapaucius` (`+§1000` each).
    AddFunds(i32),
    /// `move_objects on|off`.
    MoveObjects(bool),
    /// `map_edit on|off`.
    MapEdit(bool),
    /// `water_tool`.
    WaterTool,
    /// `autonomy <0..=100>`.
    Autonomy(u8),
    /// `interests`.
    Interests,
    /// `prepare_lot`.
    PrepareLot,
    /// `set_hour <0..=23>`.
    SetHour(u8),
    /// `sim_speed <-1000..=1000>`.
    SimSpeed(i16),
    /// `help`.
    Help,
}

/// Structured error returned when parsing a cheat console line fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheatParseError {
    Empty,
    UnknownCommand(String),
    MissingArgument {
        command: &'static str,
        expected: &'static str,
    },
    InvalidArgument {
        command: &'static str,
        expected: &'static str,
        got: String,
    },
    UnexpectedArguments {
        command: &'static str,
    },
    OutOfRange {
        command: &'static str,
        min: i32,
        max: i32,
        got: i64,
    },
    NothingToRepeat,
    ChainTooLong {
        max: usize,
    },
}

impl fmt::Display for CheatParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Enter a cheat command (type 'help' for list)."),
            Self::UnknownCommand(cmd) => write!(f, "No such cheat: {cmd}"),
            Self::MissingArgument { command, expected } => {
                write!(f, "Usage: {command} {expected}")
            }
            Self::InvalidArgument {
                command,
                expected,
                got,
            } => write!(
                f,
                "Invalid argument '{got}' for {command}; expected {expected}."
            ),
            Self::UnexpectedArguments { command } => {
                write!(f, "Command '{command}' takes no arguments.")
            }
            Self::OutOfRange {
                command,
                min,
                max,
                got,
            } => write!(f, "{command} value {got} out of range ({min}..={max})."),
            Self::NothingToRepeat => write!(f, "Nothing to repeat for '!'."),
            Self::ChainTooLong { max } => {
                write!(f, "Too many chained commands (maximum {max}).")
            }
        }
    }
}

impl std::error::Error for CheatParseError {}

/// Parses a single Sims 1 cheat console line (including `;`-separated chains
/// and `!` repetition of the previous command).
pub fn parse_cheat_line(input: &str) -> std::result::Result<Vec<CheatCommand>, CheatParseError> {
    parse_cheat_line_with_previous(input, None)
}

/// Parses a cheat console line with an optional `previous` command so a
/// standalone `!;!;` line can repeat the last executed command.
pub fn parse_cheat_line_with_previous(
    input: &str,
    previous: Option<&CheatCommand>,
) -> std::result::Result<Vec<CheatCommand>, CheatParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CheatParseError::Empty);
    }

    let mut commands = Vec::new();
    let mut last: Option<CheatCommand> = previous.cloned();

    for raw_segment in trimmed.split(';') {
        let segment = raw_segment.trim();
        if segment.is_empty() {
            continue;
        }
        if segment == "!" {
            let Some(prev) = last.clone() else {
                return Err(CheatParseError::NothingToRepeat);
            };
            if commands.len() >= MAX_CHAIN_COMMANDS {
                return Err(CheatParseError::ChainTooLong {
                    max: MAX_CHAIN_COMMANDS,
                });
            }
            commands.push(prev);
            continue;
        }
        let cmd = parse_single_command(segment)?;
        if commands.len() >= MAX_CHAIN_COMMANDS {
            return Err(CheatParseError::ChainTooLong {
                max: MAX_CHAIN_COMMANDS,
            });
        }
        last = Some(cmd.clone());
        commands.push(cmd);
    }

    if commands.is_empty() {
        return Err(CheatParseError::Empty);
    }
    Ok(commands)
}

fn parse_single_command(segment: &str) -> std::result::Result<CheatCommand, CheatParseError> {
    let mut parts = segment.split_whitespace();
    let Some(head) = parts.next() else {
        return Err(CheatParseError::Empty);
    };
    let rest: Vec<&str> = parts.collect();
    let lower = head.to_ascii_lowercase();

    match lower.as_str() {
        "rosebud" | "klapaucius" => {
            if !rest.is_empty() {
                return Err(CheatParseError::UnexpectedArguments {
                    command: if lower == "rosebud" {
                        "rosebud"
                    } else {
                        "klapaucius"
                    },
                });
            }
            Ok(CheatCommand::AddFunds(ROSEBUD_AMOUNT))
        }
        "move_objects" => parse_on_off("move_objects", &rest).map(CheatCommand::MoveObjects),
        "map_edit" => parse_on_off("map_edit", &rest).map(CheatCommand::MapEdit),
        "water_tool" => {
            if !rest.is_empty() {
                return Err(CheatParseError::UnexpectedArguments {
                    command: "water_tool",
                });
            }
            Ok(CheatCommand::WaterTool)
        }
        "autonomy" => {
            let val = parse_int_range("autonomy", &rest, 0, 100)?;
            Ok(CheatCommand::Autonomy(val as u8))
        }
        "interests" => {
            if !rest.is_empty() {
                return Err(CheatParseError::UnexpectedArguments {
                    command: "interests",
                });
            }
            Ok(CheatCommand::Interests)
        }
        "prepare_lot" => {
            if !rest.is_empty() {
                return Err(CheatParseError::UnexpectedArguments {
                    command: "prepare_lot",
                });
            }
            Ok(CheatCommand::PrepareLot)
        }
        "set_hour" => {
            let val = parse_int_range("set_hour", &rest, 0, 23)?;
            Ok(CheatCommand::SetHour(val as u8))
        }
        "sim_speed" => {
            let val = parse_int_range("sim_speed", &rest, -1000, 1000)?;
            Ok(CheatCommand::SimSpeed(val as i16))
        }
        "help" => {
            if !rest.is_empty() {
                return Err(CheatParseError::UnexpectedArguments { command: "help" });
            }
            Ok(CheatCommand::Help)
        }
        _ => Err(CheatParseError::UnknownCommand(head.to_owned())),
    }
}

fn parse_on_off(
    command: &'static str,
    args: &[&str],
) -> std::result::Result<bool, CheatParseError> {
    if args.is_empty() {
        return Err(CheatParseError::MissingArgument {
            command,
            expected: "on|off",
        });
    }
    if args.len() > 1 {
        return Err(CheatParseError::InvalidArgument {
            command,
            expected: "on|off",
            got: args.join(" "),
        });
    }
    match args[0].to_ascii_lowercase().as_str() {
        "on" | "1" | "true" => Ok(true),
        "off" | "0" | "false" => Ok(false),
        _ => Err(CheatParseError::InvalidArgument {
            command,
            expected: "on|off",
            got: args[0].to_owned(),
        }),
    }
}

fn parse_int_range(
    command: &'static str,
    args: &[&str],
    min: i32,
    max: i32,
) -> std::result::Result<i32, CheatParseError> {
    let expected = match command {
        "autonomy" => "<0..=100>",
        "set_hour" => "<0..=23>",
        "sim_speed" => "<-1000..=1000>",
        _ => "<integer>",
    };
    if args.is_empty() {
        return Err(CheatParseError::MissingArgument { command, expected });
    }
    if args.len() > 1 {
        return Err(CheatParseError::InvalidArgument {
            command,
            expected,
            got: args.join(" "),
        });
    }
    let parsed: i64 = args[0]
        .parse()
        .map_err(|_| CheatParseError::InvalidArgument {
            command,
            expected,
            got: args[0].to_owned(),
        })?;
    if parsed < i64::from(min) || parsed > i64::from(max) {
        return Err(CheatParseError::OutOfRange {
            command,
            min,
            max,
            got: parsed,
        });
    }
    Ok(parsed as i32)
}

/// Visual tone of the console's feedback line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackKind {
    Info,
    Ok,
    Error,
}

/// Status or error feedback shown directly below the console input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleFeedback {
    pub kind: FeedbackKind,
    pub message: String,
}

/// Computed screen-space geometry for [`CheatConsole`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheatConsoleLayout {
    pub bar: Rect,
    pub input_box: Rect,
    pub feedback_line: Option<Rect>,
}

/// Interactive top-left Sims 1 cheat console (`Ctrl+Shift+C`) controller.
#[derive(Debug, Clone, Default)]
pub struct CheatConsole {
    open: bool,
    input: String,
    /// Caret position in Unicode scalar values (`0..=input.chars().count()`).
    cursor: usize,
    caret_visible: bool,
    blink_since: Option<u64>,
    history: Vec<String>,
    /// Index into `history` when navigating with Up/Down (`None` = editing draft).
    history_index: Option<usize>,
    draft_backup: String,
    last_command: Option<CheatCommand>,
    feedback: Option<ConsoleFeedback>,
}

impl CheatConsole {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn feedback(&self) -> Option<&ConsoleFeedback> {
        self.feedback.as_ref()
    }

    /// Opens the cheat console and focuses the input bar.
    pub fn open(&mut self) {
        self.open = true;
        self.caret_visible = true;
        self.blink_since = None;
    }

    /// Closes the cheat console and clears transient caret state.
    pub fn close(&mut self) {
        self.open = false;
        self.caret_visible = false;
        self.blink_since = None;
    }

    /// Toggles the cheat console (the classic `Ctrl+Shift+C` hotkey).
    pub fn toggle(&mut self) -> bool {
        if self.open {
            self.close();
        } else {
            self.open();
        }
        self.open
    }

    /// Sets an external status/feedback message (e.g. after `sims-game` applies
    /// a [`CheatCommand`]).
    pub fn set_feedback(&mut self, kind: FeedbackKind, message: impl Into<String>) {
        self.feedback = Some(ConsoleFeedback {
            kind,
            message: message.into(),
        });
    }

    /// Clears the current status/error feedback line.
    pub fn clear_feedback(&mut self) {
        self.feedback = None;
    }

    fn char_to_byte_index(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map_or(self.input.len(), |(b, _)| b)
    }

    fn reset_blink(&mut self) {
        self.caret_visible = true;
        self.blink_since = None;
    }

    /// Inserts one non-control character at the caret.
    pub fn insert_char(&mut self, ch: char) -> bool {
        if !self.open || ch.is_control() || self.input.chars().count() >= MAX_INPUT_LEN {
            return false;
        }
        let byte_idx = self.char_to_byte_index(self.cursor);
        self.input.insert(byte_idx, ch);
        self.cursor += 1;
        self.history_index = None;
        self.reset_blink();
        true
    }

    /// Pastes text at the caret, skipping control characters and clamping to
    /// [`MAX_INPUT_LEN`].
    pub fn paste_text(&mut self, text: &str) -> bool {
        if !self.open {
            return false;
        }
        let mut changed = false;
        let mut count = self.input.chars().count();
        for ch in text.chars() {
            if count >= MAX_INPUT_LEN {
                break;
            }
            if !ch.is_control() {
                let byte_idx = self.char_to_byte_index(self.cursor);
                self.input.insert(byte_idx, ch);
                self.cursor += 1;
                count += 1;
                changed = true;
            }
        }
        if changed {
            self.history_index = None;
            self.reset_blink();
        }
        changed
    }

    /// Deletes the character immediately before the caret.
    pub fn backspace(&mut self) -> bool {
        if !self.open || self.cursor == 0 {
            return false;
        }
        let prev = self.cursor - 1;
        let byte_idx = self.char_to_byte_index(prev);
        self.input.remove(byte_idx);
        self.cursor = prev;
        self.history_index = None;
        self.reset_blink();
        true
    }

    /// Deletes the character immediately after the caret.
    pub fn delete_forward(&mut self) -> bool {
        let len = self.input.chars().count();
        if !self.open || self.cursor >= len {
            return false;
        }
        let byte_idx = self.char_to_byte_index(self.cursor);
        self.input.remove(byte_idx);
        self.history_index = None;
        self.reset_blink();
        true
    }

    pub fn move_left(&mut self) -> bool {
        if !self.open || self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.reset_blink();
        true
    }

    pub fn move_right(&mut self) -> bool {
        let len = self.input.chars().count();
        if !self.open || self.cursor >= len {
            return false;
        }
        self.cursor += 1;
        self.reset_blink();
        true
    }

    pub fn move_home(&mut self) -> bool {
        if !self.open || self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        self.reset_blink();
        true
    }

    pub fn move_end(&mut self) -> bool {
        let len = self.input.chars().count();
        if !self.open || self.cursor == len {
            return false;
        }
        self.cursor = len;
        self.reset_blink();
        true
    }

    /// Navigates to an older entry in command history (`Up` arrow).
    pub fn history_up(&mut self) -> bool {
        if !self.open || self.history.is_empty() {
            return false;
        }
        let next = match self.history_index {
            None => {
                self.draft_backup = self.input.clone();
                self.history.len() - 1
            }
            Some(0) => return false,
            Some(i) => i - 1,
        };
        self.history_index = Some(next);
        self.input = self.history[next].clone();
        self.cursor = self.input.chars().count();
        self.reset_blink();
        true
    }

    /// Navigates to a newer entry in command history (`Down` arrow), restoring
    /// the saved draft when stepping past the newest entry.
    pub fn history_down(&mut self) -> bool {
        if !self.open {
            return false;
        }
        let Some(current) = self.history_index else {
            return false;
        };
        if current + 1 < self.history.len() {
            let next = current + 1;
            self.history_index = Some(next);
            self.input = self.history[next].clone();
        } else {
            self.history_index = None;
            self.input = std::mem::take(&mut self.draft_backup);
        }
        self.cursor = self.input.chars().count();
        self.reset_blink();
        true
    }

    /// Advances the 500 ms `cTSWinTextEdit2` caret blink timer.
    pub fn sync_blink(&mut self, clock_us: u64) -> bool {
        if !self.open {
            let was = self.caret_visible;
            self.caret_visible = false;
            self.blink_since = None;
            return was;
        }
        let Some(since) = self.blink_since else {
            self.blink_since = Some(clock_us);
            return false;
        };
        if clock_us.saturating_sub(since) >= CARET_BLINK_US {
            self.caret_visible = !self.caret_visible;
            self.blink_since = Some(clock_us);
            return true;
        }
        false
    }

    /// Parses and submits the current input line (`Enter`), updating command
    /// history and the feedback line. Returns the parsed [`CheatCommand`] list
    /// on success.
    pub fn submit(&mut self) -> std::result::Result<Vec<CheatCommand>, CheatParseError> {
        let raw = self.input.trim().to_owned();
        let parsed = parse_cheat_line_with_previous(&raw, self.last_command.as_ref());

        if !raw.is_empty() && self.history.last().map(String::as_str) != Some(raw.as_str()) {
            self.history.push(raw);
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.history_index = None;
        self.draft_backup.clear();

        match parsed {
            Ok(commands) => {
                if let Some(last) = commands.last() {
                    self.last_command = Some(last.clone());
                }
                self.feedback = Some(summarize_commands(&commands));
                self.input.clear();
                self.cursor = 0;
                self.reset_blink();
                Ok(commands)
            }
            Err(err) => {
                self.feedback = Some(ConsoleFeedback {
                    kind: FeedbackKind::Error,
                    message: err.to_string(),
                });
                self.reset_blink();
                Err(err)
            }
        }
    }

    /// Computes top-left overlay geometry inside `frame_size`.
    pub fn layout(&self, frame_size: [u32; 2]) -> CheatConsoleLayout {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);

        let margin = 10;
        let width = 420.min((fw - margin * 2).max(160).min(fw));
        let input_h = 24;
        let has_feedback = self.feedback.is_some();
        let feedback_h = if has_feedback { 18 } else { 0 };
        let bar_h = (input_h + 12 + feedback_h).min((fh - margin * 2).max(32));

        let bar = Rect::new(margin, margin, width, bar_h);
        let input_box = Rect::new(
            bar.x + 6,
            bar.y + 6,
            (bar.width - 12).max(40),
            input_h.min((bar.height - 8).max(16)),
        );
        let feedback_line = if has_feedback && bar.height >= input_box.height + 24 {
            Some(Rect::new(
                bar.x + 8,
                input_box.y + input_box.height + 3,
                (bar.width - 16).max(40),
                16,
            ))
        } else {
            None
        };

        CheatConsoleLayout {
            bar,
            input_box,
            feedback_line,
        }
    }

    /// Draws the top-left cheat console overlay into `frame`.
    pub fn draw(&self, frame: &mut Frame, art: &Art) {
        if !self.open {
            return;
        }
        let layout = self.layout([frame.width(), frame.height()]);
        draw_console_frame(frame, art, layout.bar);

        // Input box (`cTSWinTextEdit2` dark-navy fill + cyan focus border).
        frame.fill(layout.input_box, ENTRY_FILL);
        draw_outline(frame, layout.input_box, ENTRY_FOCUS_BORDER);

        let font = art.font(10).or_else(|| art.font(12));
        let small_font = art.font(9).or_else(|| art.font(10));

        if let Some(font) = font {
            let prompt = "> ";
            let prompt_w = font.width(prompt);
            let ty = layout.input_box.y + ((layout.input_box.height - font.line_height) / 2).max(2);
            let tx = layout.input_box.x + 6;
            font.draw(frame, prompt, [tx, ty], TEXT_PROMPT);

            let text_x = tx + prompt_w;
            let avail_w = (layout.input_box.width - prompt_w - 14).max(16);
            let caret_byte = self.char_to_byte_index(self.cursor);
            let prefix = &self.input[..caret_byte];
            let visible_start = visible_window_start(font, &self.input, caret_byte, avail_w);
            let visible = fit_prefix(font, &self.input[visible_start..], avail_w);
            font.draw(frame, visible, [text_x, ty], TEXT_INPUT);

            if self.caret_visible {
                let caret_offset = font.width(&prefix[visible_start..]);
                let cx =
                    (text_x + caret_offset).min(layout.input_box.x + layout.input_box.width - 4);
                let caret_h = font
                    .line_height
                    .clamp(8, (layout.input_box.height - 6).max(8));
                frame.fill(Rect::new(cx, ty, 1, caret_h), ENTRY_FOCUS_BORDER);
            }
        } else if self.caret_visible {
            // Fallback caret pixel even when no bitmap font is loaded.
            frame.fill(
                Rect::new(
                    layout.input_box.x + 8,
                    layout.input_box.y + 5,
                    1,
                    (layout.input_box.height - 10).max(6),
                ),
                ENTRY_FOCUS_BORDER,
            );
        }

        if let (Some(fb_rect), Some(fb), Some(font)) =
            (layout.feedback_line, self.feedback.as_ref(), small_font)
        {
            let color = match fb.kind {
                FeedbackKind::Info => TEXT_INFO,
                FeedbackKind::Ok => TEXT_OK,
                FeedbackKind::Error => TEXT_ERROR,
            };
            draw_clipped_text(
                frame,
                font,
                &fb.message,
                [fb_rect.x, fb_rect.y],
                fb_rect.width,
                color,
            );
        }
    }
}

fn summarize_commands(commands: &[CheatCommand]) -> ConsoleFeedback {
    if commands.len() == 1 && commands[0] == CheatCommand::Help {
        return ConsoleFeedback {
            kind: FeedbackKind::Info,
            message: "rosebud, klapaucius, !;!, move_objects, map_edit, water_tool, autonomy, interests, prepare_lot, set_hour, sim_speed".into(),
        };
    }
    if commands
        .iter()
        .all(|c| matches!(c, CheatCommand::AddFunds(_)))
    {
        let total: i32 = commands.iter().fold(0i32, |acc, c| match c {
            CheatCommand::AddFunds(n) => acc.saturating_add(*n),
            _ => acc,
        });
        return ConsoleFeedback {
            kind: FeedbackKind::Ok,
            message: format!("Added \u{00a7}{total} ({}x).", commands.len()),
        };
    }
    if commands.len() == 1 {
        let msg = match &commands[0] {
            CheatCommand::MoveObjects(on) => {
                format!("move_objects {}", if *on { "on" } else { "off" })
            }
            CheatCommand::MapEdit(on) => format!("map_edit {}", if *on { "on" } else { "off" }),
            CheatCommand::WaterTool => "Water tool activated.".into(),
            CheatCommand::Autonomy(v) => format!("Autonomy set to {v}."),
            CheatCommand::Interests => "Showing Sim interests.".into(),
            CheatCommand::PrepareLot => "Lot prepared.".into(),
            CheatCommand::SetHour(h) => format!("Hour set to {h:02}:00."),
            CheatCommand::SimSpeed(s) => format!("Sim speed set to {s}."),
            CheatCommand::AddFunds(n) => format!("Added \u{00a7}{n}."),
            CheatCommand::Help => String::new(),
        };
        return ConsoleFeedback {
            kind: FeedbackKind::Ok,
            message: msg,
        };
    }
    ConsoleFeedback {
        kind: FeedbackKind::Ok,
        message: format!("Executed {} commands.", commands.len()),
    }
}

fn draw_console_frame(frame: &mut Frame, art: &Art, rect: Rect) {
    if let Some(tiler) = art.image(POPUP_TILER)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
    } else if let Some(tiler) = art.image(BACKGROUND)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
    } else {
        for y in rect.y..rect.y.saturating_add(rect.height) {
            for x in rect.x..rect.x.saturating_add(rect.width) {
                frame.blend(x, y, BAR_FALLBACK_BG, 220);
            }
        }
        draw_outline(frame, rect, ENTRY_BORDER);
    }
}

fn draw_outline(frame: &mut Frame, rect: Rect, color: [u8; 3]) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), color);
    frame.fill(
        Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
        color,
    );
    frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), color);
    frame.fill(
        Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
        color,
    );
}

fn visible_window_start(font: &Font, text: &str, caret_byte: usize, avail_w: i32) -> usize {
    let prefix = &text[..caret_byte];
    if font.width(prefix) <= avail_w {
        return 0;
    }
    for (idx, _) in prefix.char_indices() {
        if font.width(&prefix[idx..]) <= avail_w {
            return idx;
        }
    }
    caret_byte
}

fn fit_prefix<'a>(font: &Font, text: &'a str, max_width: i32) -> &'a str {
    if font.width(text) <= max_width {
        return text;
    }
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if font.width(&text[..next]) > max_width {
            break;
        }
        end = next;
    }
    &text[..end]
}

fn draw_clipped_text(
    frame: &mut Frame,
    font: &Font,
    text: &str,
    at: [i32; 2],
    max_width: i32,
    color: [u8; 3],
) {
    if max_width <= 0 || text.is_empty() {
        return;
    }
    if font.width(text) <= max_width {
        font.draw(frame, text, at, color);
        return;
    }
    let ellipsis = "...";
    let ellipsis_w = font.width(ellipsis);
    if ellipsis_w > max_width {
        return;
    }
    let mut clipped = String::new();
    for ch in text.chars() {
        clipped.push(ch);
        if font.width(&clipped) + ellipsis_w > max_width {
            clipped.pop();
            break;
        }
    }
    clipped.push_str(ellipsis);
    font.draw(frame, &clipped, at, color);
}
