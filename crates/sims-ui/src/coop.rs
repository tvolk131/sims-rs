//! Co-op multiplayer Host/Join lobby dialog and in-lot status/attribution
//! HUD overlay, styled with Complete Collection 1.91's native 9-slice chrome.
//!
//! Per `docs/vision.md` (**Multiplayer (v1)**), co-op runs several players on
//! one shared lot with shared money and clock:
//!
//! - **Host / Join Lobby Dialog** ([`CoopLobbyDialog`]):
//!   - **Host Session** tab: start/stop hosting, display & copy the iroh invite
//!     ticket, inspect connected players, and toggle the optional "Host-only
//!     time control" lock.
//!   - **Join Session** tab: paste/edit an invite ticket in a `cTSWinTextEdit2`
//!     field, connect/cancel, and display connection errors or file manifest
//!     mismatches.
//! - **In-Lot Co-op Status & Attribution Overlay** ([`CoopHudOverlay`]):
//!   - Compact top-right pill/panel showing connected players (with colored
//!     badge, name, and selected Sim) plus active pause reasons (`"Paused by
//!     Alice"`, `"Bob is in Buy Mode"`, `"Waiting on dialog"`).
//!   - Transient speed/pause attribution toast (`"<Player> set Ultra Speed"`).
//!   - Drift-resyncing progress banner with byte progress bar.
//!
//! Like every other `sims-ui` module, this module has zero networking or
//! simulation coupling: callers pass a read-only [`CoopSnapshot`] and execute
//! returned [`CoopCommand`] values against `sims-net`.

use crate::Result;
use crate::art::Art;
use crate::dialog::{BACKGROUND, BUTTON, CARET_BLINK_US};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::hud::rgb565;
use crate::neighborhood::{self, art_id::POPUP_TILER};
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;

/// `UIGraphics` resource IDs used by [`CoopLobbyDialog`] and [`CoopHudOverlay`]:
/// `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`), and
/// `HotspotPopupTiler.TGA` (`5107`).
pub const RESOURCES: [i32; 3] = [BACKGROUND, BUTTON, POPUP_TILER];

/// Maximum ticket length accepted in the Join Session input box (iroh node
/// tickets are typically 120–350 base32 characters).
pub const MAX_TICKET_LEN: usize = 512;

/// Loads the native 1.91 9-slice frames, button strip, and bitmap fonts used
/// by the co-op UI.
pub fn load_art(ui: &UiGraphics, language: u8) -> Result<Art> {
    Art::load(ui, language, &RESOURCES)
}

/// Standard 1.91 UI text colors (`0x6f2a94..0x6f2aa4`).
const TEXT_NORMAL: [u8; 3] = [195, 205, 205];
const TEXT_HOVER: [u8; 3] = [255, 255, 255];
const TEXT_SELECTED: [u8; 3] = [0, 255, 255];
const TEXT_DISABLED: [u8; 3] = [0x40, 0x5d, 0x5f];
const TEXT_WARNING: [u8; 3] = [255, 214, 102];
const TEXT_ERROR: [u8; 3] = [255, 118, 118];
const TEXT_OK: [u8; 3] = [120, 238, 160];

/// `cTSWinTextEdit2` background fill (`RGB(0, 0, 0x52)` quantized to RGB565).
const ENTRY_FILL: [u8; 3] = rgb565(0x000a);
const ENTRY_BORDER: [u8; 3] = [88, 120, 204];
const ENTRY_FOCUS_BORDER: [u8; 3] = [0, 255, 255];
const DIALOG_FALLBACK_BG: [u8; 3] = [24, 34, 112];
const DIALOG_FALLBACK_BORDER: [u8; 3] = [96, 132, 224];
const OVERLAY_FALLBACK_BG: [u8; 3] = [16, 26, 96];
const LIST_BG: [u8; 3] = [12, 20, 74];
const LIST_BORDER: [u8; 3] = [66, 96, 176];
const PROGRESS_TRACK: [u8; 3] = [10, 18, 64];
const PROGRESS_FILL: [u8; 3] = [44, 204, 224];

/// Opaque session-scoped player identifier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u64);

/// Local participant's role in the co-op session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CoopRole {
    #[default]
    Offline,
    Host,
    Client,
}

/// Which mode a connected player is currently viewing on the shared lot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlayerMode {
    #[default]
    Live,
    Buy,
    Build,
}

impl PlayerMode {
    /// Short badge label for non-Live modes.
    pub const fn badge(self) -> Option<&'static str> {
        match self {
            Self::Live => None,
            Self::Buy => Some("Buy"),
            Self::Build => Some("Build"),
        }
    }
}

/// Read-only summary of one player connected to the shared lot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
    pub is_host: bool,
    pub is_local: bool,
    /// Name of the Sim currently selected by this player, if any.
    pub selected_sim_name: Option<String>,
    pub mode: PlayerMode,
    /// Distinct RGB badge color for this player.
    pub badge_color: [u8; 3],
    /// Optional network RTT in milliseconds.
    pub ping_ms: Option<u16>,
}

/// Active reason why the shared simulation clock is paused (`docs/vision.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseReason {
    /// A player explicitly paused the clock.
    Explicit { player_name: String },
    /// A player is currently in Buy Mode.
    BuyMode { player_name: String },
    /// A player is currently in Build Mode.
    BuildMode { player_name: String },
    /// An unanswered blocking dialog is open on the lot.
    BlockingDialog,
}

impl PauseReason {
    /// Formats a concise user-facing pause reason string.
    pub fn label(&self) -> String {
        match self {
            Self::Explicit { player_name } => format!("Paused by {player_name}"),
            Self::BuyMode { player_name } => format!("{player_name} is in Buy Mode"),
            Self::BuildMode { player_name } => format!("{player_name} is in Build Mode"),
            Self::BlockingDialog => "Waiting on dialog".to_owned(),
        }
    }
}

/// Shared clock / speed change kind for transient attribution toasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeAction {
    Paused,
    Resumed,
    NormalSpeed,
    FastSpeed,
    UltraSpeed,
}

/// Transient speed/pause attribution toast displayed in [`CoopHudOverlay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeAttribution {
    pub player_name: String,
    pub action: TimeAction,
    /// Remaining visibility time in milliseconds (`0` hides the toast).
    pub remaining_ms: u32,
}

impl TimeAttribution {
    /// Formats the toast headline (e.g. `"Alice set Ultra Speed"`).
    pub fn label(&self) -> String {
        match self.action {
            TimeAction::Paused => format!("{} paused the game", self.player_name),
            TimeAction::Resumed => format!("{} resumed the game", self.player_name),
            TimeAction::NormalSpeed => format!("{} set Normal Speed", self.player_name),
            TimeAction::FastSpeed => format!("{} set High Speed", self.player_name),
            TimeAction::UltraSpeed => format!("{} set Ultra Speed", self.player_name),
        }
    }
}

/// Drift-recovery or mid-game REDB snapshot resync progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResyncProgress {
    pub bytes_received: u64,
    pub bytes_total: u64,
    pub reason: String,
}

impl ResyncProgress {
    /// Completion percentage clamped to `0..=100`.
    pub fn percent(&self) -> u8 {
        if self.bytes_total == 0 {
            return 0;
        }
        ((self.bytes_received.min(self.bytes_total) * 100) / self.bytes_total) as u8
    }
}

/// Connection / lobby status reported by `sims-net`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Hosting {
        invite_ticket: String,
    },
    Connecting,
    Connected,
    ManifestMismatch {
        local_summary: String,
        remote_summary: String,
        mismatched_files: Vec<String>,
    },
    Error(String),
}

/// Read-only co-op view model consumed by [`CoopLobbyDialog`] and
/// [`CoopHudOverlay`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoopSnapshot {
    pub role: CoopRole,
    pub connection: ConnectionState,
    pub players: Vec<PlayerInfo>,
    pub host_only_time_control: bool,
    pub pause_reasons: Vec<PauseReason>,
    pub time_toast: Option<TimeAttribution>,
    pub resync: Option<ResyncProgress>,
}

impl CoopSnapshot {
    /// Returns `true` when the in-lot co-op overlay has anything to display.
    pub fn is_overlay_active(&self) -> bool {
        self.role != CoopRole::Offline
            || !self.players.is_empty()
            || !self.pause_reasons.is_empty()
            || self.time_toast.as_ref().is_some_and(|t| t.remaining_ms > 0)
            || self.resync.is_some()
    }
}

/// Typed commands emitted by [`CoopLobbyDialog`] for `sims-game` / `sims-net`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoopCommand {
    /// Begin hosting a co-op session on the current lot.
    StartHosting,
    /// Stop hosting or disconnect from the active co-op session.
    Disconnect,
    /// Copy the active invite ticket to the system clipboard.
    CopyInviteTicket(String),
    /// Enable or disable the "Host-only time control" lock.
    SetHostOnlyTimeControl(bool),
    /// Connect to a hosted session using `ticket`.
    JoinSession { ticket: String },
    /// Cancel an in-progress connection attempt.
    CancelConnect,
    /// Close the Host / Join co-op dialog.
    CloseDialog,
}

/// Active tab of [`CoopLobbyDialog`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LobbyTab {
    #[default]
    Host,
    Join,
}

/// Interactive hit targets inside [`CoopLobbyDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyHit {
    HostTab,
    JoinTab,
    CloseButton,
    PrimaryButton,
    SecondaryButton,
    CopyTicketButton,
    TimeLockToggle,
    TicketInput,
}

/// Computed screen-space geometry for [`CoopLobbyDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyLayout {
    pub dialog: Rect,
    pub title_bar: Rect,
    pub host_tab: Rect,
    pub join_tab: Rect,
    pub close_btn: Rect,
    pub content: Rect,
    pub ticket_box: Option<Rect>,
    pub copy_btn: Option<Rect>,
    pub time_lock_toggle: Option<Rect>,
    pub player_list: Option<Rect>,
    pub status_box: Option<Rect>,
    pub primary_btn: Rect,
    pub secondary_btn: Option<Rect>,
}

impl LobbyLayout {
    /// Returns the interactive control under `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<LobbyHit> {
        if self.host_tab.contains(x, y) {
            return Some(LobbyHit::HostTab);
        }
        if self.join_tab.contains(x, y) {
            return Some(LobbyHit::JoinTab);
        }
        if self.close_btn.contains(x, y) {
            return Some(LobbyHit::CloseButton);
        }
        if self.copy_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(LobbyHit::CopyTicketButton);
        }
        if self.time_lock_toggle.is_some_and(|r| r.contains(x, y)) {
            return Some(LobbyHit::TimeLockToggle);
        }
        if self.ticket_box.is_some_and(|r| r.contains(x, y)) {
            return Some(LobbyHit::TicketInput);
        }
        if self.primary_btn.contains(x, y) {
            return Some(LobbyHit::PrimaryButton);
        }
        if self.secondary_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(LobbyHit::SecondaryButton);
        }
        None
    }
}

/// Interactive Host / Join Co-op Lobby dialog controller.
#[derive(Debug, Clone, Default)]
pub struct CoopLobbyDialog {
    open: bool,
    tab: LobbyTab,
    ticket_input: String,
    input_focused: bool,
    caret_visible: bool,
    blink_since: Option<u64>,
    copied_feedback: bool,
    hovered: Option<LobbyHit>,
    pressed: Option<LobbyHit>,
}

impl CoopLobbyDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn tab(&self) -> LobbyTab {
        self.tab
    }

    pub fn ticket_input(&self) -> &str {
        &self.ticket_input
    }

    pub fn is_input_focused(&self) -> bool {
        self.input_focused
    }

    pub fn copied_feedback(&self) -> bool {
        self.copied_feedback
    }

    /// Opens the dialog on `tab`, resetting transient pointer capture state.
    pub fn open(&mut self, tab: LobbyTab) {
        self.open = true;
        self.tab = tab;
        self.input_focused = tab == LobbyTab::Join;
        self.caret_visible = self.input_focused;
        self.blink_since = None;
        self.copied_feedback = false;
        self.hovered = None;
        self.pressed = None;
    }

    /// Closes the dialog and clears transient pointer state.
    pub fn close(&mut self) {
        self.open = false;
        self.input_focused = false;
        self.caret_visible = false;
        self.blink_since = None;
        self.hovered = None;
        self.pressed = None;
    }

    /// Switches between Host and Join tabs.
    pub fn set_tab(&mut self, tab: LobbyTab) {
        if self.tab != tab {
            self.tab = tab;
            self.input_focused = tab == LobbyTab::Join;
            self.caret_visible = self.input_focused;
            self.blink_since = None;
            self.copied_feedback = false;
            self.pressed = None;
        }
    }

    /// Clears the Join ticket input buffer.
    pub fn clear_ticket(&mut self) {
        self.ticket_input.clear();
        self.reset_blink();
    }

    /// Inserts one character into the Join ticket input when on the Join tab
    /// and focused.
    pub fn insert_char(&mut self, ch: char) -> bool {
        if !self.open || self.tab != LobbyTab::Join || !self.input_focused {
            return false;
        }
        if ch.is_control() || self.ticket_input.chars().count() >= MAX_TICKET_LEN {
            return false;
        }
        self.ticket_input.push(ch);
        self.reset_blink();
        true
    }

    /// Pastes a ticket string into the Join input field, stripping whitespace
    /// and control characters and clamping to [`MAX_TICKET_LEN`].
    pub fn paste_text(&mut self, text: &str) -> bool {
        if !self.open || self.tab != LobbyTab::Join {
            return false;
        }
        self.input_focused = true;
        let mut changed = false;
        let mut count = self.ticket_input.chars().count();
        for ch in text.chars() {
            if count >= MAX_TICKET_LEN {
                break;
            }
            if !ch.is_control() && !ch.is_whitespace() {
                self.ticket_input.push(ch);
                count += 1;
                changed = true;
            }
        }
        if changed {
            self.reset_blink();
        }
        changed
    }

    /// Deletes the last character from the Join ticket input.
    pub fn backspace(&mut self) -> bool {
        if !self.open || self.tab != LobbyTab::Join || !self.input_focused {
            return false;
        }
        if self.ticket_input.pop().is_some() {
            self.reset_blink();
            true
        } else {
            false
        }
    }

    /// Advances the 500 ms `cTSWinTextEdit2` caret blink timer.
    pub fn sync_blink(&mut self, clock_us: u64) -> bool {
        if !self.open || self.tab != LobbyTab::Join || !self.input_focused {
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

    fn reset_blink(&mut self) {
        self.caret_visible = true;
        self.blink_since = None;
    }

    /// Handles `Enter` key submission on the active tab.
    pub fn submit(&mut self, snapshot: &CoopSnapshot) -> Option<CoopCommand> {
        if !self.open {
            return None;
        }
        match self.tab {
            LobbyTab::Host => match &snapshot.connection {
                ConnectionState::Hosting { .. } => None,
                _ => Some(CoopCommand::StartHosting),
            },
            LobbyTab::Join => {
                if matches!(snapshot.connection, ConnectionState::Connecting) {
                    return None;
                }
                let trimmed = self.ticket_input.trim();
                if trimmed.is_empty() {
                    return None;
                }
                Some(CoopCommand::JoinSession {
                    ticket: trimmed.to_owned(),
                })
            }
        }
    }

    /// Handles `Escape` key press (cancels active connect or closes dialog).
    pub fn escape(&mut self, snapshot: &CoopSnapshot) -> Option<CoopCommand> {
        if !self.open {
            return None;
        }
        if self.tab == LobbyTab::Join && matches!(snapshot.connection, ConnectionState::Connecting)
        {
            return Some(CoopCommand::CancelConnect);
        }
        self.close();
        Some(CoopCommand::CloseDialog)
    }

    /// Computes responsive dialog geometry centered inside `frame_size`.
    pub fn layout(&self, frame_size: [u32; 2], snapshot: &CoopSnapshot) -> LobbyLayout {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);

        let margin = 12;
        let max_w = (fw - margin * 2).max(200).min(fw);
        let max_h = (fh - margin * 2).max(180).min(fh);
        let width = 460.min(max_w);
        let height = 340.min(max_h);
        let x = ((fw - width) / 2).max(0);
        let y = ((fh - height) / 2).max(0);
        let dialog = Rect::new(x, y, width, height);

        let pad = 16;
        let inner_x = dialog.x + pad;
        let inner_w = (dialog.width - pad * 2).max(120);

        let title_bar = Rect::new(inner_x, dialog.y + 12, inner_w, 22);

        // Top tab bar + Close button on the right.
        let tab_y = title_bar.y + title_bar.height + 6;
        let tab_h = 28;
        let gap = 6;
        let close_w = 64.min((inner_w / 4).max(40));
        let tabs_avail = (inner_w - close_w - gap * 2).max(0);
        let tab_w = ((tabs_avail - gap) / 2).max(0);
        let host_tab = Rect::new(inner_x, tab_y, tab_w, tab_h);
        let join_tab = Rect::new(host_tab.x + host_tab.width + gap, tab_y, tab_w, tab_h);
        let close_btn = Rect::new(inner_x + inner_w - close_w, tab_y, close_w, tab_h);

        // Footer action buttons.
        let btn_h = 30;
        let btn_y = (dialog.y + dialog.height - pad - btn_h).max(tab_y + tab_h + 24);
        let content_top = tab_y + tab_h + 10;
        let content_h = (btn_y - content_top - 8).max(40);
        let content = Rect::new(inner_x, content_top, inner_w, content_h);

        let (primary_btn, secondary_btn) = match self.tab {
            LobbyTab::Host => {
                let w = 136.min(inner_w);
                (Rect::new(inner_x + inner_w - w, btn_y, w, btn_h), None)
            }
            LobbyTab::Join => {
                let w = 112.min((inner_w - gap) / 2);
                let primary = Rect::new(inner_x + inner_w - w, btn_y, w, btn_h);
                let secondary = Rect::new(primary.x - gap - w, btn_y, w, btn_h);
                (primary, Some(secondary))
            }
        };

        let mut ticket_box = None;
        let mut copy_btn = None;
        let mut time_lock_toggle = None;
        let mut player_list = None;
        let mut status_box = None;

        match self.tab {
            LobbyTab::Host => {
                let mut cur_y = content.y;
                if matches!(snapshot.connection, ConnectionState::Hosting { .. }) {
                    // Invite ticket row: read-only box + Copy button.
                    let label_h = 16;
                    cur_y += label_h;
                    let row_h = 28;
                    let copy_w = 88.min((inner_w / 3).max(64));
                    let box_w = (inner_w - copy_w - gap).max(60);
                    ticket_box = Some(Rect::new(inner_x, cur_y, box_w, row_h));
                    copy_btn = Some(Rect::new(inner_x + box_w + gap, cur_y, copy_w, row_h));
                    cur_y += row_h + 8;

                    // Host-only time control checkbox row.
                    let toggle_h = 22;
                    time_lock_toggle = Some(Rect::new(inner_x, cur_y, inner_w, toggle_h));
                    cur_y += toggle_h + 8;

                    // Player list fills remaining content area.
                    let list_h = (content.y + content.height - cur_y).max(36);
                    player_list = Some(Rect::new(inner_x, cur_y, inner_w, list_h));
                } else {
                    // Not hosting yet: info/status box explaining hosting.
                    status_box = Some(content);
                }
            }
            LobbyTab::Join => {
                let label_h = 16;
                let input_y = content.y + label_h;
                let input_h = 30;
                ticket_box = Some(Rect::new(inner_x, input_y, inner_w, input_h));
                let status_y = input_y + input_h + 10;
                let status_h = (content.y + content.height - status_y).max(28);
                status_box = Some(Rect::new(inner_x, status_y, inner_w, status_h));
            }
        }

        LobbyLayout {
            dialog,
            title_bar,
            host_tab,
            join_tab,
            close_btn,
            content,
            ticket_box,
            copy_btn,
            time_lock_toggle,
            player_list,
            status_box,
            primary_btn,
            secondary_btn,
        }
    }

    /// Updates hover state for `(x, y)`. Returns `true` if inside the dialog.
    pub fn pointer_move(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &CoopSnapshot,
        x: i32,
        y: i32,
    ) -> bool {
        if !self.open {
            return false;
        }
        let layout = self.layout(frame_size, snapshot);
        self.hovered = layout.hit(x, y);
        layout.dialog.contains(x, y)
    }

    /// Handles mouse press at `(x, y)`. Returns `true` if consumed by the dialog.
    pub fn pointer_down(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &CoopSnapshot,
        x: i32,
        y: i32,
    ) -> bool {
        if !self.open {
            return false;
        }
        let layout = self.layout(frame_size, snapshot);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        self.pressed = hit;
        if self.tab == LobbyTab::Join {
            self.input_focused = hit == Some(LobbyHit::TicketInput);
            if self.input_focused {
                self.reset_blink();
            }
        }
        layout.dialog.contains(x, y)
    }

    /// Handles mouse release at `(x, y)` and returns any resulting [`CoopCommand`].
    pub fn pointer_up(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &CoopSnapshot,
        x: i32,
        y: i32,
    ) -> Option<CoopCommand> {
        if !self.open {
            return None;
        }
        let layout = self.layout(frame_size, snapshot);
        let hit = layout.hit(x, y);
        self.hovered = hit;
        let pressed = self.pressed.take();
        if pressed != hit {
            return None;
        }
        match hit? {
            LobbyHit::HostTab => {
                self.set_tab(LobbyTab::Host);
                None
            }
            LobbyHit::JoinTab => {
                self.set_tab(LobbyTab::Join);
                None
            }
            LobbyHit::CloseButton => {
                self.close();
                Some(CoopCommand::CloseDialog)
            }
            LobbyHit::CopyTicketButton => {
                if let ConnectionState::Hosting { invite_ticket } = &snapshot.connection {
                    self.copied_feedback = true;
                    Some(CoopCommand::CopyInviteTicket(invite_ticket.clone()))
                } else {
                    None
                }
            }
            LobbyHit::TimeLockToggle => {
                if matches!(snapshot.connection, ConnectionState::Hosting { .. }) {
                    Some(CoopCommand::SetHostOnlyTimeControl(
                        !snapshot.host_only_time_control,
                    ))
                } else {
                    None
                }
            }
            LobbyHit::TicketInput => {
                if self.tab == LobbyTab::Join {
                    self.input_focused = true;
                    self.reset_blink();
                }
                None
            }
            LobbyHit::PrimaryButton => match self.tab {
                LobbyTab::Host => match &snapshot.connection {
                    ConnectionState::Hosting { .. } => Some(CoopCommand::Disconnect),
                    _ => Some(CoopCommand::StartHosting),
                },
                LobbyTab::Join => {
                    if matches!(snapshot.connection, ConnectionState::Connecting) {
                        return None;
                    }
                    let trimmed = self.ticket_input.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(CoopCommand::JoinSession {
                            ticket: trimmed.to_owned(),
                        })
                    }
                }
            },
            LobbyHit::SecondaryButton => {
                if self.tab == LobbyTab::Join {
                    if matches!(snapshot.connection, ConnectionState::Connecting) {
                        Some(CoopCommand::CancelConnect)
                    } else {
                        self.close();
                        Some(CoopCommand::CloseDialog)
                    }
                } else {
                    None
                }
            }
        }
    }

    /// Draws the Host / Join Co-op Lobby dialog into `frame` when open.
    pub fn draw(&self, frame: &mut Frame, snapshot: &CoopSnapshot, art: &Art) {
        if !self.open {
            return;
        }
        let layout = self.layout([frame.width(), frame.height()], snapshot);
        draw_dialog_frame(frame, art, layout.dialog);

        let title_font = art.font(12).or_else(|| art.font(10));
        let body_font = art.font(10).or_else(|| art.font(12));

        // Title bar.
        if let Some(font) = title_font {
            let title = match self.tab {
                LobbyTab::Host => "Co-op Multiplayer - Host Session",
                LobbyTab::Join => "Co-op Multiplayer - Join Session",
            };
            draw_clipped_text(
                frame,
                font,
                title,
                [layout.title_bar.x, layout.title_bar.y + 2],
                layout.title_bar.width,
                TEXT_HOVER,
            );
        }

        // Tabs + Close button.
        draw_button(
            frame,
            art,
            layout.host_tab,
            "Host",
            self.tab_button_state(LobbyHit::HostTab, self.tab == LobbyTab::Host),
        );
        draw_button(
            frame,
            art,
            layout.join_tab,
            "Join",
            self.tab_button_state(LobbyHit::JoinTab, self.tab == LobbyTab::Join),
        );
        draw_button(
            frame,
            art,
            layout.close_btn,
            "Close",
            self.button_state(LobbyHit::CloseButton, true),
        );

        match self.tab {
            LobbyTab::Host => self.draw_host_body(frame, snapshot, art, &layout, body_font),
            LobbyTab::Join => self.draw_join_body(frame, snapshot, art, &layout, body_font),
        }

        // Footer buttons.
        match self.tab {
            LobbyTab::Host => {
                let (label, enabled) = match &snapshot.connection {
                    ConnectionState::Hosting { .. } => ("Stop Hosting", true),
                    _ => ("Start Hosting", true),
                };
                draw_button(
                    frame,
                    art,
                    layout.primary_btn,
                    label,
                    self.button_state(LobbyHit::PrimaryButton, enabled),
                );
            }
            LobbyTab::Join => {
                let can_connect = !self.ticket_input.trim().is_empty()
                    && !matches!(snapshot.connection, ConnectionState::Connecting);
                draw_button(
                    frame,
                    art,
                    layout.primary_btn,
                    if matches!(snapshot.connection, ConnectionState::Connecting) {
                        "Connecting..."
                    } else {
                        "Connect"
                    },
                    self.button_state(LobbyHit::PrimaryButton, can_connect),
                );
                if let Some(sec) = layout.secondary_btn {
                    draw_button(
                        frame,
                        art,
                        sec,
                        "Cancel",
                        self.button_state(LobbyHit::SecondaryButton, true),
                    );
                }
            }
        }
    }

    fn draw_host_body(
        &self,
        frame: &mut Frame,
        snapshot: &CoopSnapshot,
        art: &Art,
        layout: &LobbyLayout,
        body_font: Option<&Font>,
    ) {
        if let ConnectionState::Hosting { invite_ticket } = &snapshot.connection {
            if let (Some(box_rect), Some(font)) = (layout.ticket_box, body_font) {
                draw_clipped_text(
                    frame,
                    font,
                    "Invite Ticket:",
                    [box_rect.x, box_rect.y - 16],
                    layout.content.width,
                    TEXT_NORMAL,
                );
                draw_text_box(frame, box_rect, false);
                draw_clipped_text(
                    frame,
                    font,
                    invite_ticket,
                    [box_rect.x + 6, box_rect.y + 7],
                    (box_rect.width - 12).max(16),
                    TEXT_SELECTED,
                );
            } else if let Some(box_rect) = layout.ticket_box {
                draw_text_box(frame, box_rect, false);
            }

            if let Some(copy_rect) = layout.copy_btn {
                let copy_label = if self.copied_feedback {
                    "Copied!"
                } else {
                    "Copy"
                };
                draw_button(
                    frame,
                    art,
                    copy_rect,
                    copy_label,
                    self.button_state(LobbyHit::CopyTicketButton, true),
                );
            }

            if let Some(toggle_rect) = layout.time_lock_toggle {
                let cb = Rect::new(toggle_rect.x, toggle_rect.y + 3, 16, 16);
                frame.fill(cb, ENTRY_FILL);
                let border = if self.hovered == Some(LobbyHit::TimeLockToggle) {
                    TEXT_HOVER
                } else {
                    ENTRY_BORDER
                };
                draw_outline(frame, cb, border);
                if snapshot.host_only_time_control {
                    frame.fill(Rect::new(cb.x + 3, cb.y + 3, 10, 10), TEXT_SELECTED);
                }
                if let Some(font) = body_font {
                    draw_clipped_text(
                        frame,
                        font,
                        "Host-only time control",
                        [cb.x + 24, toggle_rect.y + 4],
                        (toggle_rect.width - 24).max(16),
                        if snapshot.host_only_time_control {
                            TEXT_HOVER
                        } else {
                            TEXT_NORMAL
                        },
                    );
                }
            }

            if let Some(list_rect) = layout.player_list {
                frame.fill(list_rect, LIST_BG);
                draw_outline(frame, list_rect, LIST_BORDER);
                if let Some(font) = body_font {
                    let lh = (font.line_height + 4).max(16);
                    let mut row_y = list_rect.y + 6;
                    let header = format!("Connected Players ({})", snapshot.players.len());
                    draw_clipped_text(
                        frame,
                        font,
                        &header,
                        [list_rect.x + 8, row_y],
                        (list_rect.width - 16).max(16),
                        TEXT_HOVER,
                    );
                    row_y += lh;
                    let max_y = list_rect.y + list_rect.height - 4;
                    for p in &snapshot.players {
                        if row_y + font.line_height > max_y {
                            break;
                        }
                        frame.fill(Rect::new(list_rect.x + 8, row_y + 3, 8, 8), p.badge_color);
                        let mut line = p.name.clone();
                        if p.is_host {
                            line.push_str(" (Host)");
                        }
                        if let Some(sim) = &p.selected_sim_name {
                            line.push_str(&format!(" - {sim}"));
                        }
                        if let Some(ping) = p.ping_ms {
                            line.push_str(&format!(" [{ping}ms]"));
                        }
                        draw_clipped_text(
                            frame,
                            font,
                            &line,
                            [list_rect.x + 22, row_y],
                            (list_rect.width - 30).max(16),
                            TEXT_NORMAL,
                        );
                        row_y += lh;
                    }
                }
            }
        } else if let Some(status_rect) = layout.status_box {
            frame.fill(status_rect, LIST_BG);
            draw_outline(frame, status_rect, LIST_BORDER);
            if let Some(font) = body_font {
                let text = match &snapshot.connection {
                    ConnectionState::Error(msg) => format!("Host error: {msg}"),
                    _ => "Start hosting to share this lot over iroh. Friends join with your invite ticket.".to_owned(),
                };
                let color = if matches!(snapshot.connection, ConnectionState::Error(_)) {
                    TEXT_ERROR
                } else {
                    TEXT_NORMAL
                };
                draw_wrapped_text(
                    frame,
                    font,
                    &text,
                    Rect::new(
                        status_rect.x + 10,
                        status_rect.y + 10,
                        (status_rect.width - 20).max(20),
                        (status_rect.height - 20).max(16),
                    ),
                    color,
                );
            }
        }
    }

    fn draw_join_body(
        &self,
        frame: &mut Frame,
        snapshot: &CoopSnapshot,
        _art: &Art,
        layout: &LobbyLayout,
        body_font: Option<&Font>,
    ) {
        if let Some(box_rect) = layout.ticket_box {
            if let Some(font) = body_font {
                draw_clipped_text(
                    frame,
                    font,
                    "Paste Invite Ticket:",
                    [box_rect.x, box_rect.y - 16],
                    layout.content.width,
                    TEXT_NORMAL,
                );
            }
            draw_text_box(frame, box_rect, self.input_focused);
            if let Some(font) = body_font {
                let avail_w = (box_rect.width - 14).max(16);
                let visible = tail_fitting(font, &self.ticket_input, avail_w);
                let text_x = box_rect.x + 6;
                let text_y = box_rect.y + ((box_rect.height - font.line_height) / 2).max(2);
                if self.ticket_input.is_empty() && !self.input_focused {
                    draw_clipped_text(
                        frame,
                        font,
                        "iroh://...",
                        [text_x, text_y],
                        avail_w,
                        TEXT_DISABLED,
                    );
                } else {
                    font.draw(frame, visible, [text_x, text_y], TEXT_HOVER);
                    if self.input_focused && self.caret_visible {
                        let cx =
                            (text_x + font.width(visible)).min(box_rect.x + box_rect.width - 5);
                        frame.fill(
                            Rect::new(cx, box_rect.y + 5, 2, (box_rect.height - 10).max(4)),
                            TEXT_SELECTED,
                        );
                    }
                }
            }
        }

        if let Some(status_rect) = layout.status_box {
            frame.fill(status_rect, LIST_BG);
            draw_outline(frame, status_rect, LIST_BORDER);
            if let Some(font) = body_font {
                let inner = Rect::new(
                    status_rect.x + 10,
                    status_rect.y + 10,
                    (status_rect.width - 20).max(20),
                    (status_rect.height - 20).max(16),
                );
                match &snapshot.connection {
                    ConnectionState::Disconnected => {
                        draw_wrapped_text(
                            frame,
                            font,
                            "Paste a host's invite ticket above and press Connect.",
                            inner,
                            TEXT_NORMAL,
                        );
                    }
                    ConnectionState::Connecting => {
                        draw_wrapped_text(
                            frame,
                            font,
                            "Connecting to host and verifying game file manifest...",
                            inner,
                            TEXT_SELECTED,
                        );
                    }
                    ConnectionState::Connected | ConnectionState::Hosting { .. } => {
                        draw_wrapped_text(
                            frame,
                            font,
                            "Connected to co-op session.",
                            inner,
                            TEXT_OK,
                        );
                    }
                    ConnectionState::ManifestMismatch {
                        local_summary,
                        remote_summary,
                        mismatched_files,
                    } => {
                        let mut msg = format!(
                            "Manifest mismatch! Local: {local_summary} | Host: {remote_summary}"
                        );
                        if !mismatched_files.is_empty() {
                            let sample: Vec<&str> = mismatched_files
                                .iter()
                                .take(3)
                                .map(String::as_str)
                                .collect();
                            msg.push_str(&format!(" ({})", sample.join(", ")));
                        }
                        draw_wrapped_text(frame, font, &msg, inner, TEXT_WARNING);
                    }
                    ConnectionState::Error(err) => {
                        draw_wrapped_text(
                            frame,
                            font,
                            &format!("Connection failed: {err}"),
                            inner,
                            TEXT_ERROR,
                        );
                    }
                }
            }
        }
    }

    fn button_state(&self, hit: LobbyHit, enabled: bool) -> ButtonState {
        if !enabled {
            ButtonState::Disabled
        } else if self.pressed == Some(hit) && self.hovered == Some(hit) {
            ButtonState::Selected
        } else if self.hovered == Some(hit) {
            ButtonState::Hover
        } else {
            ButtonState::Normal
        }
    }

    fn tab_button_state(&self, hit: LobbyHit, active: bool) -> ButtonState {
        if active || (self.pressed == Some(hit) && self.hovered == Some(hit)) {
            ButtonState::Selected
        } else if self.hovered == Some(hit) {
            ButtonState::Hover
        } else {
            ButtonState::Normal
        }
    }
}

/// Computed screen-space geometry for [`CoopHudOverlay`], guaranteed
/// non-overlapping across viewports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayLayout {
    /// Top-right connected players + active pause reasons pill/panel.
    pub status_panel: Option<Rect>,
    /// Transient speed/pause attribution toast pill.
    pub time_toast: Option<Rect>,
    /// Drift-resyncing progress banner.
    pub resync_banner: Option<Rect>,
}

/// Stateless in-lot co-op status, pause reason, speed attribution toast, and
/// drift-resyncing overlay renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoopHudOverlay;

impl CoopHudOverlay {
    /// Computes strictly disjoint rectangles for the active overlay elements
    /// inside `frame_size`.
    pub fn layout(frame_size: [u32; 2], snapshot: &CoopSnapshot, art: &Art) -> OverlayLayout {
        if !snapshot.is_overlay_active() {
            return OverlayLayout::default();
        }
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);
        let font = art.font(10).or_else(|| art.font(12));
        let lh = font.map_or(14, |f| f.line_height.max(12));
        let margin = 10;
        let gap = 8;

        // 1. Top-right connected players & pause reasons panel.
        let status_panel = if !snapshot.players.is_empty()
            || !snapshot.pause_reasons.is_empty()
            || snapshot.role != CoopRole::Offline
        {
            let panel_w = 232.min((fw - margin * 2).max(140));
            let header_rows = 1;
            let player_rows = snapshot.players.len().min(6) as i32;
            let pause_rows = snapshot.pause_reasons.len().min(3) as i32;
            let extra_gap = if pause_rows > 0 { 6 } else { 0 };
            let panel_h = (16 + (header_rows + player_rows + pause_rows) * (lh + 2) + extra_gap)
                .min((fh - margin * 2).max(36));
            let px = (fw - margin - panel_w).max(margin);
            Some(Rect::new(px, margin, panel_w, panel_h))
        } else {
            None
        };

        // 2. Transient speed/pause attribution toast.
        let time_toast = snapshot
            .time_toast
            .as_ref()
            .filter(|t| t.remaining_ms > 0)
            .map(|toast| {
                let label = toast.label();
                let text_w = font.map_or(label.len() as i32 * 7, |f| f.width(&label));
                let toast_h = (lh + 14).max(28);
                let max_center_w = if let Some(sp) = status_panel {
                    // Available width centered left of the top-right status panel.
                    (sp.x - margin - gap).max(0)
                } else {
                    (fw - margin * 2).max(80)
                };
                if max_center_w >= 160 {
                    let w = (text_w + 28).clamp(140, max_center_w.min(340));
                    let x = ((fw - w) / 2).clamp(margin, (max_center_w + margin - w).max(margin));
                    Rect::new(x, margin, w, toast_h)
                } else {
                    // Narrow viewport: stack cleanly below status_panel.
                    let w = (fw - margin * 2).clamp(80, 300);
                    let y = status_panel.map_or(margin, |sp| sp.y + sp.height + gap);
                    Rect::new(((fw - w) / 2).max(margin), y, w, toast_h)
                }
            });

        // 3. Drift-resyncing progress banner, stacked below any overlapping top element.
        let resync_banner = snapshot.resync.as_ref().map(|_| {
            let banner_w = 320.min((fw - margin * 2).max(160));
            let banner_h = (lh * 2 + 28).max(56);
            let bx = ((fw - banner_w) / 2).max(margin);
            let mut by = margin;
            if let Some(toast) = time_toast {
                by = by.max(toast.y + toast.height + gap);
            }
            if let Some(sp) = status_panel {
                let candidate = Rect::new(bx, by, banner_w, banner_h);
                if rects_intersect(candidate, sp) {
                    by = sp.y + sp.height + gap;
                }
            }
            // Never clamp upward into `time_toast` or `status_panel`; shrink
            // height if the viewport is extremely short.
            let max_h = (fh - by - margin).max(0);
            Rect::new(bx, by, banner_w, banner_h.min(max_h))
        });

        OverlayLayout {
            status_panel,
            time_toast,
            resync_banner,
        }
    }

    /// Draws the active co-op HUD overlay elements into `frame`.
    pub fn draw(frame: &mut Frame, snapshot: &CoopSnapshot, art: &Art) {
        if !snapshot.is_overlay_active() {
            return;
        }
        let layout = Self::layout([frame.width(), frame.height()], snapshot, art);
        let font = art.font(10).or_else(|| art.font(12));
        let lh = font.map_or(14, |f| f.line_height.max(12));

        // 1. Connected players + pause reasons pill/panel.
        if let Some(rect) = layout.status_panel {
            draw_overlay_pill(frame, art, rect);
            if let Some(font) = font {
                let mut y = rect.y + 8;
                let max_y = rect.y + rect.height - 6;
                let inner_w = (rect.width - 16).max(20);
                let header = if snapshot.host_only_time_control {
                    format!("Co-op ({}) [Time Locked]", snapshot.players.len())
                } else {
                    format!("Co-op ({})", snapshot.players.len())
                };
                draw_clipped_text(frame, font, &header, [rect.x + 8, y], inner_w, TEXT_HOVER);
                y += lh + 2;

                for player in snapshot.players.iter().take(6) {
                    if y + lh > max_y {
                        break;
                    }
                    frame.fill(Rect::new(rect.x + 8, y + 3, 7, 7), player.badge_color);
                    let mut text = player.name.clone();
                    if let Some(sim) = &player.selected_sim_name {
                        text.push_str(&format!(" [{sim}]"));
                    }
                    if let Some(badge) = player.mode.badge() {
                        text.push_str(&format!(" ({badge})"));
                    }
                    draw_clipped_text(
                        frame,
                        font,
                        &text,
                        [rect.x + 20, y],
                        (inner_w - 12).max(16),
                        if player.is_local {
                            TEXT_SELECTED
                        } else {
                            TEXT_NORMAL
                        },
                    );
                    y += lh + 2;
                }

                if !snapshot.pause_reasons.is_empty() && y + lh <= max_y {
                    y += 4;
                    for reason in snapshot.pause_reasons.iter().take(3) {
                        if y + lh > max_y {
                            break;
                        }
                        draw_clipped_text(
                            frame,
                            font,
                            &reason.label(),
                            [rect.x + 8, y],
                            inner_w,
                            TEXT_WARNING,
                        );
                        y += lh + 2;
                    }
                }
            }
        }

        // 2. Transient speed / pause attribution toast.
        if let (Some(rect), Some(toast)) = (layout.time_toast, snapshot.time_toast.as_ref()) {
            draw_overlay_pill(frame, art, rect);
            if let Some(font) = font {
                let label = toast.label();
                let ty = rect.y + ((rect.height - font.line_height) / 2).max(2);
                let avail_w = (rect.width - 16).max(16);
                let w = font.width(&label).min(avail_w);
                let tx = rect.x + ((rect.width - w) / 2).max(8);
                draw_clipped_text(frame, font, &label, [tx, ty], avail_w, TEXT_SELECTED);
            }
        }

        // 3. Drift-resyncing progress banner.
        if let (Some(rect), Some(resync)) = (layout.resync_banner, snapshot.resync.as_ref()) {
            draw_dialog_frame(frame, art, rect);
            let inner_x = rect.x + 12;
            let inner_w = (rect.width - 24).max(20);
            if let Some(font) = font {
                let pct = resync.percent();
                let title = format!("Resyncing session... {pct}%");
                draw_clipped_text(
                    frame,
                    font,
                    &title,
                    [inner_x, rect.y + 8],
                    inner_w,
                    TEXT_WARNING,
                );
                draw_clipped_text(
                    frame,
                    font,
                    &resync.reason,
                    [inner_x, rect.y + 8 + lh + 2],
                    inner_w,
                    TEXT_NORMAL,
                );
            }
            let bar_h = 8;
            let bar_y = (rect.y + rect.height - bar_h - 10).max(rect.y + 8);
            let bar = Rect::new(inner_x, bar_y, inner_w, bar_h);
            frame.fill(bar, PROGRESS_TRACK);
            draw_outline(frame, bar, ENTRY_BORDER);
            let fill_w = if resync.bytes_total > 0 {
                let ratio = (resync.bytes_received.min(resync.bytes_total) as u128
                    * (inner_w - 2).max(0) as u128)
                    / resync.bytes_total as u128;
                ratio as i32
            } else {
                0
            };
            if fill_w > 0 {
                frame.fill(
                    Rect::new(bar.x + 1, bar.y + 1, fill_w, (bar.height - 2).max(1)),
                    PROGRESS_FILL,
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonState {
    Normal,
    Selected,
    Hover,
    Disabled,
}

impl ButtonState {
    const fn cell(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Selected => 1,
            Self::Hover => 2,
            Self::Disabled => 3,
        }
    }

    const fn text_color(self) -> [u8; 3] {
        match self {
            Self::Normal => TEXT_NORMAL,
            Self::Selected => TEXT_SELECTED,
            Self::Hover => TEXT_HOVER,
            Self::Disabled => TEXT_DISABLED,
        }
    }
}

fn draw_dialog_frame(frame: &mut Frame, art: &Art, rect: Rect) {
    if let Some(tiler) = art.image(BACKGROUND)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
    } else {
        frame.fill(rect, DIALOG_FALLBACK_BG);
        draw_outline(frame, rect, DIALOG_FALLBACK_BORDER);
    }
}

fn draw_overlay_pill(frame: &mut Frame, art: &Art, rect: Rect) {
    if let Some(tiler) = art.image(POPUP_TILER)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
    } else {
        for y in rect.y..rect.y.saturating_add(rect.height) {
            for x in rect.x..rect.x.saturating_add(rect.width) {
                frame.blend(x, y, OVERLAY_FALLBACK_BG, 215);
            }
        }
        draw_outline(frame, rect, ENTRY_BORDER);
    }
}

fn draw_text_box(frame: &mut Frame, rect: Rect, focused: bool) {
    frame.fill(rect, ENTRY_FILL);
    draw_outline(
        frame,
        rect,
        if focused {
            ENTRY_FOCUS_BORDER
        } else {
            ENTRY_BORDER
        },
    );
}

fn draw_button(frame: &mut Frame, art: &Art, rect: Rect, label: &str, state: ButtonState) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    if let Some(img) = art.image(BUTTON)
        && img.width >= 4
        && img.height > 0
    {
        let cw = (img.width / 4) as i32;
        let ch = img.height as i32;
        let sx = cw.saturating_mul(state.cell() as i32);
        draw_three_slice_h(frame, img, Rect::new(sx, 0, cw, ch), rect);
    } else {
        let fill = match state {
            ButtonState::Normal => [38, 60, 148],
            ButtonState::Selected => [22, 112, 168],
            ButtonState::Hover => [64, 96, 196],
            ButtonState::Disabled => [34, 42, 74],
        };
        frame.fill(rect, fill);
        draw_outline(frame, rect, ENTRY_BORDER);
    }

    if let Some(font) = art.font(10).or_else(|| art.font(12)) {
        let avail_w = (rect.width - 10).max(8);
        let text_w = font.width(label).min(avail_w);
        let tx = rect.x + ((rect.width - text_w) / 2).max(5);
        let ty = rect.y + ((rect.height - font.line_height) / 2).max(2);
        draw_clipped_text(frame, font, label, [tx, ty], avail_w, state.text_color());
    }
}

/// Draws a button cell horizontally 3-sliced (preserving left/right caps) and
/// vertically scaled to `dest`.
fn draw_three_slice_h(frame: &mut Frame, image: &Image, src: Rect, dest: Rect) {
    let cap = (src.width / 4).min(dest.width / 2).max(1);
    let mid_src_w = (src.width - cap * 2).max(1);
    let mid_dst_w = (dest.width - cap * 2).max(0);

    draw_scaled_region(
        frame,
        image,
        Rect::new(src.x, src.y, cap, src.height),
        Rect::new(dest.x, dest.y, cap, dest.height),
    );
    if mid_dst_w > 0 {
        draw_scaled_region(
            frame,
            image,
            Rect::new(src.x + cap, src.y, mid_src_w, src.height),
            Rect::new(dest.x + cap, dest.y, mid_dst_w, dest.height),
        );
    }
    draw_scaled_region(
        frame,
        image,
        Rect::new(src.x + src.width - cap, src.y, cap, src.height),
        Rect::new(dest.x + dest.width - cap, dest.y, cap, dest.height),
    );
}

fn draw_scaled_region(frame: &mut Frame, image: &Image, src: Rect, dest: Rect) {
    if src.width <= 0 || src.height <= 0 || dest.width <= 0 || dest.height <= 0 {
        return;
    }
    let fw = i32::try_from(frame.width()).unwrap_or(i32::MAX);
    let fh = i32::try_from(frame.height()).unwrap_or(i32::MAX);
    let x0 = dest.x.max(0);
    let y0 = dest.y.max(0);
    let x1 = dest.x.saturating_add(dest.width).min(fw);
    let y1 = dest.y.saturating_add(dest.height).min(fh);
    for y in y0..y1 {
        let sy = src.y
            + ((i64::from(y - dest.y) * i64::from(src.height)) / i64::from(dest.height)) as i32;
        for x in x0..x1 {
            let sx = src.x
                + ((i64::from(x - dest.x) * i64::from(src.width)) / i64::from(dest.width)) as i32;
            if let (Ok(ux), Ok(uy)) = (u32::try_from(sx), u32::try_from(sy))
                && let Some([r, g, b, a]) = image.pixel(ux, uy)
            {
                frame.blend(x, y, [r, g, b], a);
            }
        }
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

fn tail_fitting<'a>(font: &Font, text: &'a str, max_width: i32) -> &'a str {
    if font.width(text) <= max_width {
        return text;
    }
    for (idx, _) in text.char_indices() {
        let slice = &text[idx..];
        if font.width(slice) <= max_width {
            return slice;
        }
    }
    ""
}

fn draw_wrapped_text(frame: &mut Frame, font: &Font, text: &str, rect: Rect, color: [u8; 3]) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    let lh = font.line_height.max(1);
    let mut y = rect.y;
    let max_y = rect.y + rect.height;
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && font.width(&candidate) > rect.width {
            if y + lh > max_y {
                return;
            }
            draw_clipped_text(frame, font, &line, [rect.x, y], rect.width, color);
            y += lh;
            line.clear();
            line.push_str(word);
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() && y + lh <= max_y {
        draw_clipped_text(frame, font, &line, [rect.x, y], rect.width, color);
    }
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    let ax1 = i64::from(a.x) + i64::from(a.width);
    let ay1 = i64::from(a.y) + i64::from(a.height);
    let bx1 = i64::from(b.x) + i64::from(b.width);
    let by1 = i64::from(b.y) + i64::from(b.height);
    i64::from(a.x) < bx1 && ax1 > i64::from(b.x) && i64::from(a.y) < by1 && ay1 > i64::from(b.y)
}
