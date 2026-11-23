//! Travel shop, wardrobe, pet adoption, and Magic Town vendor modal picker,
//! styled with Complete Collection 1.91's native 9-slice chrome (`3008`, `17`,
//! `5107`).
//!
//! Covers all Complete Collection destination & vendor pickers:
//! - Downtown boutiques (formal, swimwear, pajamas, winterwear, everyday)
//! - Vacation Island souvenir and rental kiosks
//! - Unleashed pet adoption displays
//! - Studio Town wardrobe racks
//! - Magic Town spell, ingredient, and MagiCoin vendors
//!
//! Localized labels are loaded at runtime from `GameData/UIText.iff` (`STR#`
//! 142, 220, 221) via [`ShopStrings::load`]; no verbatim game prose is embedded
//! in source or tests.

use crate::art::Art;
use crate::dialog::{BACKGROUND, BUTTON};
use crate::font::Font;
use crate::frame::{Frame, Rect};
use crate::neighborhood::{self, art_id::POPUP_TILER};
use crate::{Error, Result};
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_formats::iff::Iff;
use sims_formats::strings::StringTable;

/// `kClothingDialogPrevious` (`Res_CPanel.h`,
/// `Nbhd/DesignCharSkinsLeftBtn.bmp`, four 18×34 cells).
pub const CLOTHING_PREVIOUS: i32 = 300;
/// `kClothingDialogNext` (`Nbhd/DesignCharSkinsRightBtn.bmp`).
pub const CLOTHING_NEXT: i32 = 301;

/// `UIGraphics` resource IDs used by [`ShopPickerDialog`]:
/// `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`),
/// `HotspotPopupTiler.TGA` (`5107`), and the choice dialogs' arrows.
pub const RESOURCES: [i32; 5] = [
    BACKGROUND,
    BUTTON,
    POPUP_TILER,
    CLOTHING_PREVIOUS,
    CLOTHING_NEXT,
];

/// The 3D view of the native choice dialogs (`0x509779`: 200 by 300).
pub const CHOICE_VIEW: [u32; 2] = [200, 300];

/// `GameData/UIText.iff` default dialog button labels (`STR# 142`).
pub const DIALOG_LABELS_STR: u16 = 142;
/// `GameData/UIText.iff` clothing / wardrobe picker labels (`STR# 220`).
pub const CLOTHING_STR: u16 = 220;
/// `GameData/UIText.iff` pet adoption picker labels (`STR# 221`).
pub const PET_STR: u16 = 221;

/// Loads the native 1.91 9-slice frames, button strip, and bitmap fonts used
/// by [`ShopPickerDialog`].
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
const TEXT_MAGIC: [u8; 3] = [208, 152, 255];

const DIALOG_FALLBACK_BG: [u8; 3] = [24, 34, 112];
const DIALOG_FALLBACK_BORDER: [u8; 3] = [96, 132, 224];
const LIST_BG: [u8; 3] = [12, 20, 74];
const LIST_BORDER: [u8; 3] = [66, 96, 176];
const CARD_BG: [u8; 3] = [18, 28, 92];
const CARD_HOVER_BG: [u8; 3] = [26, 44, 122];
const CARD_SELECTED_BG: [u8; 3] = [24, 72, 148];
const ICON_SLOT_BG: [u8; 3] = [10, 16, 56];
const DETAIL_BG: [u8; 3] = [14, 22, 82];

/// Card row height in pixels inside the scrollable item list.
const CARD_HEIGHT: i32 = 54;
/// Width of the vertical scroll button column when items overflow.
const SCROLL_BTN_WIDTH: i32 = 22;

/// Wardrobe category for Downtown boutiques and Studio Town racks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WardrobeCategory {
    #[default]
    Formal,
    Swimwear,
    Pajamas,
    Winterwear,
    Everyday,
}

/// The native choice dialogs, which show one 3D preview between two
/// arrows instead of a list (see [`ShopPickerDialog::layout`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceDialog {
    /// `cWinClothingChoiceDlg` (`0x5090d0`, `STR# 220`): Wardrobe and
    /// Charm Transform.
    Clothing,
    /// The pet choice dialog (`0x521911`, `STR# 221`).
    Pet,
}

/// Destination / vendor picker variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopKind {
    DowntownBoutique(WardrobeCategory),
    VacationKiosk,
    PetAdoption,
    StudioWardrobe,
    MagicVendor,
}

impl Default for ShopKind {
    fn default() -> Self {
        Self::DowntownBoutique(WardrobeCategory::Formal)
    }
}

/// Currency accepted for a shop item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CurrencyKind {
    /// Standard Simoleons (`§`).
    #[default]
    Simoleons,
    /// Magic Town MagiCoins (`MC`).
    MagiCoins,
}

impl CurrencyKind {
    /// Formats a non-negative price in this currency (`§120` or `25 MC`).
    pub fn format_price(self, amount: i32) -> String {
        let clamped = amount.max(0);
        match self {
            Self::Simoleons => format!("\u{00a7}{clamped}"),
            Self::MagiCoins => format!("{clamped} MC"),
        }
    }
}

/// Localized labels read from `GameData/UIText.iff` (`STR#` 142, 220, 221) with
/// generic technical fallbacks when an install is not present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopStrings {
    /// The choice dialogs' accept button: standard label 3 (`0x5e5214`,
    /// "OK", `STR# 142` entry 0).
    pub ok: String,
    pub buy_clothing: String,
    pub buy_pet: String,
    pub buy_generic: String,
    pub try_on: String,
    pub cancel: String,
    pub prev_clothing: String,
    pub next_clothing: String,
    pub prev_pet: String,
    pub next_pet: String,
    pub title_wardrobe: String,
    pub title_pajamas: String,
    pub title_swimwear: String,
    pub title_formalwear: String,
    pub title_winterwear: String,
    pub title_vacation: String,
    pub title_pet: String,
    pub title_studio: String,
    pub title_magic: String,
}

impl Default for ShopStrings {
    fn default() -> Self {
        Self {
            ok: "OK".into(),
            buy_clothing: "Buy".into(),
            buy_pet: "Adopt".into(),
            buy_generic: "Buy".into(),
            try_on: "Try On".into(),
            cancel: "Cancel".into(),
            prev_clothing: "Prev".into(),
            next_clothing: "Next".into(),
            prev_pet: "Prev".into(),
            next_pet: "Next".into(),
            title_wardrobe: "Wardrobe".into(),
            title_pajamas: "Sleepwear".into(),
            title_swimwear: "Swimwear".into(),
            title_formalwear: "Formalwear".into(),
            title_winterwear: "Winterwear".into(),
            title_vacation: "Vacation Kiosk".into(),
            title_pet: "Pet Adoption".into(),
            title_studio: "Studio Wardrobe".into(),
            title_magic: "Magic Vendor".into(),
        }
    }
}

impl ShopStrings {
    /// Loads localized shop, wardrobe, and pet adoption labels from
    /// `GameData/UIText.iff` for `language` (1 = US English).
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install
            .file("GameData/UIText.iff")
            .map_err(Error::Content)?;
        let bytes = std::fs::read(&path).map_err(Error::Io)?;
        let iff = Iff::parse(bytes).map_err(Error::Format)?;
        let mut strings = Self::default();

        let read_entry = |table: &StringTable, index: usize| -> Option<String> {
            table
                .get(index, language)
                .or_else(|| table.get(index, 1))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };

        if let Some(chunk) = iff.get(b"STR#", DIALOG_LABELS_STR)
            && let Ok(table) = StringTable::parse(iff.data(chunk))
        {
            if let Some(ok) = read_entry(&table, 0) {
                strings.ok = ok;
            }
            if let Some(cancel) = read_entry(&table, 1) {
                strings.cancel = cancel;
            }
        }

        if let Some(chunk) = iff.get(b"STR#", CLOTHING_STR)
            && let Ok(table) = StringTable::parse(iff.data(chunk))
        {
            if let Some(s) = read_entry(&table, 0) {
                strings.next_clothing = s;
            }
            if let Some(s) = read_entry(&table, 1) {
                strings.prev_clothing = s;
            }
            if let Some(s) = read_entry(&table, 2) {
                strings.buy_clothing = s;
            }
            if let Some(s) = read_entry(&table, 3) {
                strings.cancel = s;
            }
            if let Some(s) = read_entry(&table, 4) {
                strings.title_wardrobe = s;
            }
            if let Some(s) = read_entry(&table, 5) {
                strings.title_pajamas = s;
            }
            if let Some(s) = read_entry(&table, 6) {
                strings.title_swimwear = s;
            }
            if let Some(s) = read_entry(&table, 7) {
                strings.title_formalwear = s;
            }
        }

        if let Some(chunk) = iff.get(b"STR#", PET_STR)
            && let Ok(table) = StringTable::parse(iff.data(chunk))
        {
            if let Some(s) = read_entry(&table, 0) {
                strings.next_pet = s;
            }
            if let Some(s) = read_entry(&table, 1) {
                strings.prev_pet = s;
            }
            if let Some(s) = read_entry(&table, 2) {
                strings.buy_pet = s;
            }
            if let Some(s) = read_entry(&table, 4) {
                strings.title_pet = s;
            }
        }

        Ok(strings)
    }

    /// Returns the localized dialog title for `kind`.
    pub fn title_for(&self, kind: ShopKind) -> &str {
        match kind {
            ShopKind::DowntownBoutique(WardrobeCategory::Formal) => &self.title_formalwear,
            ShopKind::DowntownBoutique(WardrobeCategory::Swimwear) => &self.title_swimwear,
            ShopKind::DowntownBoutique(WardrobeCategory::Pajamas) => &self.title_pajamas,
            ShopKind::DowntownBoutique(WardrobeCategory::Winterwear) => &self.title_winterwear,
            ShopKind::DowntownBoutique(WardrobeCategory::Everyday) => &self.title_wardrobe,
            ShopKind::VacationKiosk => &self.title_vacation,
            ShopKind::PetAdoption => &self.title_pet,
            ShopKind::StudioWardrobe => &self.title_studio,
            ShopKind::MagicVendor => &self.title_magic,
        }
    }

    /// The title of a choice dialog: entry 4 of its table, whatever the
    /// wardrobe mode (`0x50914a` reads entries 0 to 4 only).
    pub fn choice_title(&self, choice: ChoiceDialog) -> &str {
        match choice {
            ChoiceDialog::Clothing => &self.title_wardrobe,
            ChoiceDialog::Pet => &self.title_pet,
        }
    }

    /// Returns the localized Buy / Adopt button label for `kind`.
    pub fn buy_label_for(&self, kind: ShopKind) -> &str {
        match kind {
            ShopKind::DowntownBoutique(_) | ShopKind::StudioWardrobe => &self.buy_clothing,
            ShopKind::PetAdoption => &self.buy_pet,
            ShopKind::VacationKiosk | ShopKind::MagicVendor => &self.buy_generic,
        }
    }
}

/// Household currency balances used for live affordability checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Wallet {
    pub simoleons: i32,
    pub magic_coins: i32,
}

impl Wallet {
    pub const fn balance(self, currency: CurrencyKind) -> i32 {
        match currency {
            CurrencyKind::Simoleons => self.simoleons,
            CurrencyKind::MagiCoins => self.magic_coins,
        }
    }

    /// Returns `true` when `price >= 0` and the household has enough funds in
    /// `currency`.
    pub const fn can_afford(self, price: i32, currency: CurrencyKind) -> bool {
        price >= 0 && self.balance(currency) >= price
    }
}

/// One purchasable or adoptable catalog card displayed in [`ShopPickerDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopItem {
    /// Opaque item/outfit/pet/ingredient ID passed back in [`ShopCommand`].
    pub id: String,
    pub name: String,
    pub description: String,
    /// Cost in `currency` (negative values are rejected as unpurchasable).
    pub price: i32,
    pub currency: CurrencyKind,
    /// Optional thumbnail/portrait bitmap drawn aspect-preserving in the card.
    pub icon: Option<Image>,
    /// Whether this item supports the `Try On` preview action (e.g. outfits).
    pub supports_try_on: bool,
    /// Whether the household/Sim already owns this item.
    pub owned: bool,
    /// Whether the item is currently in stock / eligible for purchase.
    pub available: bool,
}

/// Read-only view model for [`ShopPickerDialog`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShopSnapshot {
    pub kind: ShopKind,
    /// Optional explicit title overriding [`ShopStrings::title_for`].
    pub title_override: Option<String>,
    /// Optional explicit Buy/Confirm button label overriding [`ShopStrings::buy_label_for`].
    pub buy_label_override: Option<String>,
    /// Optional name of the shopping Sim.
    pub shopper_name: Option<String>,
    pub wallet: Wallet,
    pub items: Vec<ShopItem>,
    /// Currently previewed item ID, if the Sim is trying something on.
    pub tried_on_item_id: Option<String>,
    /// Live 3D Sim turntable preview image rendered for Try-On.
    pub preview_image: Option<Image>,
    /// Optional caption or fallback status shown inside the 3D preview pane.
    pub preview_label: Option<String>,
    /// When `true`, hides the Simoleon/MagiCoin wallet bar, per-item price tags,
    /// and disabled `Try On` button so non-monetary pickers (e.g. `.FAM` family
    /// import or free charm/wardrobe selectors) never display a bogus `$0` wallet.
    pub hide_wallet_and_prices: bool,
    /// Draws the native choice dialog instead of the list.
    pub choice: Option<ChoiceDialog>,
    /// A choice dialog's body shapes, one per item: the three characters
    /// of its suit name after the first six (`b004mafit_01` → `fit`), which
    /// the clothing dialog's arrows match against the shown item's
    /// (`0x40d1e5`, `0x40d24f`, compared with `strcmp` at `0x40d280`).
    /// Empty when the arrows skip nothing: the pet and Charm dialogs.
    pub body_shapes: Vec<String>,
}

/// Typed commands emitted by [`ShopPickerDialog`] for `sims-game` / `sims-vm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShopCommand {
    /// Preview the selected item (e.g. outfit try-on).
    TryOn { item_id: String },
    /// Rotate the live 3D Try-On Sim preview by `delta_deg` degrees.
    RotatePreview(i16),
    /// Purchase or adopt the selected item.
    Buy {
        item_id: String,
        currency: CurrencyKind,
        price: i32,
    },
    /// Close the shop picker without purchasing.
    Cancel,
    /// A choice dialog's OK while none of its items is in the view: native
    /// answers index `-1` (`0x5098b3`: `0x56d852` finds no item equal to the
    /// view's outfit). What `-1` means is the caller's: the pet dialog's
    /// `-1` makes Make New Character draw a random breed
    /// (`0x44b064..0x44b09b`).
    NoSelection,
}

/// Interactive hit targets inside [`ShopPickerDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopHit {
    ItemCard(usize),
    /// A choice dialog's previous arrow (`kClothingDialogPrevious`).
    PreviousItem,
    /// A choice dialog's next arrow (`kClothingDialogNext`).
    NextItem,
    ScrollUp,
    ScrollDown,
    RotateLeft,
    RotateRight,
    TryOnButton,
    BuyButton,
    CancelButton,
}

/// Computed screen-space geometry for [`ShopPickerDialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShopLayout {
    pub dialog: Rect,
    pub title_bar: Rect,
    pub wallet_bar: Rect,
    pub list_area: Rect,
    /// Visible item cards `(item_index, card_rect)`.
    pub visible_cards: Vec<(usize, Rect)>,
    pub scroll_up: Option<Rect>,
    pub scroll_down: Option<Rect>,
    /// Optional right-hand 3D Sim Try-On preview pane.
    pub preview_pane: Option<Rect>,
    pub rotate_left_btn: Option<Rect>,
    pub rotate_right_btn: Option<Rect>,
    /// A choice dialog's arrows either side of its 3D view.
    pub previous_btn: Option<Rect>,
    pub next_btn: Option<Rect>,
    pub detail_bar: Rect,
    pub try_on_btn: Rect,
    pub buy_btn: Rect,
    pub cancel_btn: Rect,
}

impl ShopLayout {
    /// Returns the interactive control under `(x, y)`, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<ShopHit> {
        if self.scroll_up.is_some_and(|r| r.contains(x, y)) {
            return Some(ShopHit::ScrollUp);
        }
        if self.scroll_down.is_some_and(|r| r.contains(x, y)) {
            return Some(ShopHit::ScrollDown);
        }
        if self.rotate_left_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(ShopHit::RotateLeft);
        }
        if self.rotate_right_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(ShopHit::RotateRight);
        }
        if self.previous_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(ShopHit::PreviousItem);
        }
        if self.next_btn.is_some_and(|r| r.contains(x, y)) {
            return Some(ShopHit::NextItem);
        }
        for &(idx, rect) in &self.visible_cards {
            if rect.contains(x, y) {
                return Some(ShopHit::ItemCard(idx));
            }
        }
        if self.try_on_btn.contains(x, y) {
            return Some(ShopHit::TryOnButton);
        }
        if self.buy_btn.contains(x, y) {
            return Some(ShopHit::BuyButton);
        }
        if self.cancel_btn.contains(x, y) {
            return Some(ShopHit::CancelButton);
        }
        None
    }
}

/// Interactive Travel Shop, Wardrobe, Pet Adoption & Magic Vendor picker.
#[derive(Debug, Clone, Default)]
pub struct ShopPickerDialog {
    open: bool,
    selected_item_id: Option<String>,
    scroll_offset: usize,
    follow_selection: bool,
    hovered: Option<ShopHit>,
    pressed: Option<ShopHit>,
    pressed_item: Option<(String, CurrencyKind, i32)>,
}

impl ShopPickerDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Opens the picker dialog and resets transient pointer/scroll state.
    pub fn open(&mut self) {
        self.open = true;
        self.scroll_offset = 0;
        self.follow_selection = false;
        self.hovered = None;
        self.pressed = None;
        self.pressed_item = None;
    }

    /// Closes the picker dialog and clears transient pointer state.
    pub fn close(&mut self) {
        self.open = false;
        self.follow_selection = false;
        self.hovered = None;
        self.pressed = None;
        self.pressed_item = None;
    }

    /// Selects an item by ID.
    pub fn select_item(&mut self, item_id: impl Into<String>) {
        self.selected_item_id = Some(item_id.into());
        self.follow_selection = true;
    }

    /// Chooses no item. A choice dialog then shows the person as they
    /// are, its OK answers [`ShopCommand::NoSelection`], and its arrows
    /// start from before the first item (`0x40d1bb`: next from `-1` is
    /// item 0; previous wraps to the last).
    pub fn clear_selection(&mut self) {
        self.selected_item_id = None;
        self.follow_selection = false;
    }

    /// The item a choice dialog shows, if any (no fallback to the first).
    pub fn chosen_item<'a>(&self, snapshot: &'a ShopSnapshot) -> Option<&'a ShopItem> {
        let id = self.selected_item_id.as_deref()?;
        snapshot.items.iter().find(|i| i.id == id)
    }

    /// Returns the currently selected item, defaulting to the first item when
    /// none is selected or the stored ID is no longer present.
    pub fn selected_item<'a>(&self, snapshot: &'a ShopSnapshot) -> Option<&'a ShopItem> {
        if let Some(id) = self.selected_item_id.as_deref()
            && let Some(item) = snapshot.items.iter().find(|i| i.id == id)
        {
            return Some(item);
        }
        snapshot.items.first()
    }

    fn selected_index(&self, snapshot: &ShopSnapshot) -> Option<usize> {
        if snapshot.items.is_empty() {
            return None;
        }
        if let Some(id) = self.selected_item_id.as_deref()
            && let Some(idx) = snapshot.items.iter().position(|i| i.id == id)
        {
            return Some(idx);
        }
        Some(0)
    }

    fn effective_scroll(&self, snapshot: &ShopSnapshot, capacity: usize) -> usize {
        let max_scroll = snapshot.items.len().saturating_sub(capacity);
        let mut scroll = self.scroll_offset.min(max_scroll);
        if self.follow_selection
            && let Some(idx) = self.selected_index(snapshot)
        {
            if idx < scroll {
                scroll = idx;
            } else if idx >= scroll + capacity {
                scroll = (idx + 1).saturating_sub(capacity).min(max_scroll);
            }
        }
        scroll
    }

    /// Moves selection to the previous item.
    pub fn select_prev(&mut self, snapshot: &ShopSnapshot) -> bool {
        let Some(idx) = self.selected_index(snapshot) else {
            return false;
        };
        if idx == 0 {
            return false;
        }
        let next = idx - 1;
        self.selected_item_id = Some(snapshot.items[next].id.clone());
        self.follow_selection = true;
        if next < self.scroll_offset {
            self.scroll_offset = next;
        }
        true
    }

    /// Moves selection to the next item.
    pub fn select_next(&mut self, snapshot: &ShopSnapshot) -> bool {
        let Some(idx) = self.selected_index(snapshot) else {
            return false;
        };
        if idx + 1 >= snapshot.items.len() {
            return false;
        }
        let next = idx + 1;
        self.selected_item_id = Some(snapshot.items[next].id.clone());
        self.follow_selection = true;
        true
    }

    /// Scrolls the item list by `delta_rows` (positive scrolls down).
    pub fn scroll(
        &mut self,
        delta_rows: i32,
        frame_size: [u32; 2],
        snapshot: &ShopSnapshot,
    ) -> bool {
        let layout = self.layout(frame_size, snapshot);
        let capacity = (layout.list_area.height / CARD_HEIGHT).max(1) as usize;
        let max_scroll = snapshot.items.len().saturating_sub(capacity);
        let current = self.effective_scroll(snapshot, capacity);
        self.follow_selection = false;
        let next = if delta_rows >= 0 {
            current.saturating_add(delta_rows as usize).min(max_scroll)
        } else {
            current.saturating_sub(delta_rows.unsigned_abs() as usize)
        };
        if next != self.scroll_offset {
            self.scroll_offset = next;
            true
        } else {
            false
        }
    }

    /// Returns `true` when the selected item supports Try-On and is available.
    pub fn can_try_on_selected(&self, snapshot: &ShopSnapshot) -> bool {
        self.selected_item(snapshot).is_some_and(|item| {
            !item.id.trim().is_empty() && item.available && item.supports_try_on
        })
    }

    /// Returns `true` when the selected item is available, not already owned,
    /// has a non-negative price, and is affordable in its currency.
    pub fn can_buy_selected(&self, snapshot: &ShopSnapshot) -> bool {
        self.selected_item(snapshot).is_some_and(|item| {
            !item.id.trim().is_empty()
                && item.available
                && !item.owned
                && snapshot.wallet.can_afford(item.price, item.currency)
        })
    }

    /// Human-readable affordability / status reason for the selected item.
    pub fn selected_status_hint(&self, snapshot: &ShopSnapshot) -> Option<&'static str> {
        let item = self.selected_item(snapshot)?;
        if !item.available {
            return Some("Currently unavailable");
        }
        if item.owned {
            return Some("Already owned");
        }
        if item.price < 0 {
            return Some("Invalid price");
        }
        if !snapshot.wallet.can_afford(item.price, item.currency) {
            return Some(match item.currency {
                CurrencyKind::Simoleons => "Insufficient Simoleons",
                CurrencyKind::MagiCoins => "Insufficient MagiCoins",
            });
        }
        None
    }

    /// Handles `Enter` key submission (buys the selected item if affordable).
    pub fn submit(&self, snapshot: &ShopSnapshot) -> Option<ShopCommand> {
        if !self.open {
            return None;
        }
        if snapshot.choice.is_some() && self.chosen_item(snapshot).is_none() {
            return Some(ShopCommand::NoSelection);
        }
        if !self.can_buy_selected(snapshot) {
            return None;
        }
        let item = self.selected_item(snapshot)?;
        Some(ShopCommand::Buy {
            item_id: item.id.clone(),
            currency: item.currency,
            price: item.price,
        })
    }

    /// Handles `Escape` key press (closes dialog and emits `Cancel`).
    pub fn escape(&mut self) -> Option<ShopCommand> {
        if !self.open {
            return None;
        }
        self.close();
        Some(ShopCommand::Cancel)
    }

    /// Computes responsive dialog geometry centered inside `frame_size`.
    pub fn layout(&self, frame_size: [u32; 2], snapshot: &ShopSnapshot) -> ShopLayout {
        let fw = i32::try_from(frame_size[0]).unwrap_or(800).max(1);
        let fh = i32::try_from(frame_size[1]).unwrap_or(600).max(1);
        if snapshot.choice.is_some() {
            return Self::choice_layout(fw, fh);
        }

        let margin = 12;
        let max_w = (fw - margin * 2).max(220).min(fw);
        let max_h = (fh - margin * 2).max(200).min(fh);
        let wants_preview = snapshot.preview_image.is_some() || snapshot.tried_on_item_id.is_some();
        let show_preview = wants_preview && max_w >= 380;
        let width = if show_preview {
            680.min(max_w)
        } else {
            540.min(max_w)
        };
        let height = 420.min(max_h);
        let x = ((fw - width) / 2).max(0);
        let y = ((fh - height) / 2).max(0);
        let dialog = Rect::new(x, y, width, height);

        let pad = 16;
        let inner_x = dialog.x + pad;
        let inner_w = (dialog.width - pad * 2).max(120);

        let title_bar = Rect::new(inner_x, dialog.y + 12, inner_w, 20);
        let wallet_bar = if snapshot.hide_wallet_and_prices {
            Rect::new(inner_x, title_bar.y + title_bar.height, inner_w, 0)
        } else {
            Rect::new(inner_x, title_bar.y + title_bar.height + 2, inner_w, 18)
        };

        let footer_h = 30;
        let footer_y = dialog.y + dialog.height - footer_h - 14;
        let gap = 8;
        let hide_try_on =
            snapshot.hide_wallet_and_prices && !snapshot.items.iter().any(|i| i.supports_try_on);
        let (try_on_btn, buy_btn, cancel_btn) = if hide_try_on {
            let btn_w = ((inner_w - gap) / 2).max(48);
            let buy_btn = Rect::new(inner_x, footer_y, btn_w, footer_h);
            let cancel_x = inner_x + btn_w + gap;
            let cancel_btn = Rect::new(
                cancel_x,
                footer_y,
                (inner_x + inner_w - cancel_x).max(48),
                footer_h,
            );
            (Rect::new(inner_x, footer_y, 0, 0), buy_btn, cancel_btn)
        } else {
            let btn_w = ((inner_w - gap * 2) / 3).max(48);
            let try_on_btn = Rect::new(inner_x, footer_y, btn_w, footer_h);
            let buy_btn = Rect::new(inner_x + btn_w + gap, footer_y, btn_w, footer_h);
            let cancel_x = inner_x + (btn_w + gap) * 2;
            let cancel_btn = Rect::new(
                cancel_x,
                footer_y,
                (inner_x + inner_w - cancel_x).max(48),
                footer_h,
            );
            (try_on_btn, buy_btn, cancel_btn)
        };

        let detail_h = if dialog.height < 260 { 22 } else { 32 };
        let detail_bar = Rect::new(inner_x, footer_y - detail_h - 8, inner_w, detail_h);

        let list_y = wallet_bar.y + wallet_bar.height + 6;
        let list_h = (detail_bar.y - list_y - 6).max(CARD_HEIGHT);
        let (list_area, preview_pane, rotate_left_btn, rotate_right_btn) = if show_preview {
            let preview_w = 136.min((inner_w / 3).max(96));
            let list_w = (inner_w - preview_w - gap).max(160);
            let list_area = Rect::new(inner_x, list_y, list_w, list_h);
            let pane = Rect::new(
                inner_x + list_w + gap,
                list_y,
                (inner_w - list_w - gap).max(72),
                list_h,
            );
            let rot_h = 22.min((pane.height / 4).max(16));
            let rot_y = pane.y + pane.height - rot_h - 4;
            let rot_w = ((pane.width - 12) / 2).max(20);
            let left = Rect::new(pane.x + 4, rot_y, rot_w, rot_h);
            let right = Rect::new(pane.x + pane.width - rot_w - 4, rot_y, rot_w, rot_h);
            (list_area, Some(pane), Some(left), Some(right))
        } else {
            (
                Rect::new(inner_x, list_y, inner_w, list_h),
                None,
                None,
                None,
            )
        };

        let capacity = (list_area.height / CARD_HEIGHT).max(1) as usize;
        let needs_scroll = snapshot.items.len() > capacity;
        let scroll = self.effective_scroll(snapshot, capacity);

        let card_w = if needs_scroll {
            (list_area.width - SCROLL_BTN_WIDTH - 4).max(60)
        } else {
            (list_area.width - 4).max(60)
        };

        let mut visible_cards = Vec::new();
        for vis in 0..capacity {
            let idx = scroll + vis;
            if idx >= snapshot.items.len() {
                break;
            }
            let cy = list_area.y + 2 + vis as i32 * CARD_HEIGHT;
            let ch = (CARD_HEIGHT - 2).min(list_area.y + list_area.height - 2 - cy);
            if ch > 0 {
                visible_cards.push((idx, Rect::new(list_area.x + 2, cy, card_w, ch)));
            }
        }

        let (scroll_up, scroll_down) = if needs_scroll {
            let sx = list_area.x + list_area.width - SCROLL_BTN_WIDTH - 1;
            let sh = (list_area.height / 2 - 3).clamp(16, 26);
            (
                Some(Rect::new(sx, list_area.y + 2, SCROLL_BTN_WIDTH, sh)),
                Some(Rect::new(
                    sx,
                    list_area.y + list_area.height - sh - 2,
                    SCROLL_BTN_WIDTH,
                    sh,
                )),
            )
        } else {
            (None, None)
        };

        ShopLayout {
            dialog,
            title_bar,
            wallet_bar,
            list_area,
            visible_cards,
            scroll_up,
            scroll_down,
            preview_pane,
            rotate_left_btn,
            rotate_right_btn,
            previous_btn: None,
            next_btn: None,
            detail_bar,
            try_on_btn,
            buy_btn,
            cancel_btn,
        }
    }

    /// The native choice dialogs' geometry (`0x5092b2..0x5097c4`): the
    /// title, a 200 by 300 3D view centred in the area between the title
    /// and the buttons, an 18 by 34 arrow 9 pixels outside each side of it,
    /// centred on it, and OK then Cancel along the bottom, 9 pixels in.
    ///
    /// **Unverified:** the dialog's own size and its frame's border, which
    /// the original takes from the window it opens in; here the area is
    /// just big enough for the view and arrows.
    fn choice_layout(fw: i32, fh: i32) -> ShopLayout {
        let [view_w, view_h] = CHOICE_VIEW.map(|v| v as i32);
        let (arrow_w, arrow_h) = (18, 34);
        let inset = 9;
        let pad = 16;
        let title_h = 20;
        let footer_h = 30;
        // Title, then 18 above the view area (`[ebp-0x20]`), and 18 above
        // the buttons below it (`[ebp-0x18]`).
        let width = view_w + 2 * (arrow_w + 2 * inset) + 2 * pad;
        let height = 12 + title_h + 18 + view_h + 18 + footer_h + 14;
        let dialog = Rect::new(
            ((fw - width) / 2).max(0),
            ((fh - height) / 2).max(0),
            width,
            height,
        );
        let inner_x = dialog.x + pad;
        let inner_w = dialog.width - 2 * pad;
        let title_bar = Rect::new(inner_x, dialog.y + 12, inner_w, title_h);
        let top = title_bar.y + title_h + 18;
        let view = Rect::new(dialog.x + (dialog.width - view_w) / 2, top, view_w, view_h);
        let arrow_y = view.y + (view_h - arrow_h) / 2;
        let previous_btn = Rect::new(view.x - inset - arrow_w, arrow_y, arrow_w, arrow_h);
        let next_btn = Rect::new(view.x + view_w + inset, arrow_y, arrow_w, arrow_h);
        let footer_y = view.y + view_h + 18;
        let gap = 15;
        let btn_w = ((inner_w - gap) / 2).max(48);
        let buy_btn = Rect::new(inner_x, footer_y, btn_w, footer_h);
        let cancel_btn = Rect::new(inner_x + btn_w + gap, footer_y, btn_w, footer_h);
        let none = Rect::new(inner_x, footer_y, 0, 0);
        ShopLayout {
            dialog,
            title_bar,
            wallet_bar: none,
            list_area: none,
            visible_cards: Vec::new(),
            scroll_up: None,
            scroll_down: None,
            preview_pane: Some(view),
            rotate_left_btn: None,
            rotate_right_btn: None,
            previous_btn: Some(previous_btn),
            next_btn: Some(next_btn),
            detail_bar: none,
            try_on_btn: none,
            buy_btn,
            cancel_btn,
        }
    }

    /// Updates hover state for `(x, y)`.
    pub fn pointer_move(&mut self, frame_size: [u32; 2], snapshot: &ShopSnapshot, x: i32, y: i32) {
        if !self.open {
            self.hovered = None;
            return;
        }
        let layout = self.layout(frame_size, snapshot);
        self.hovered = layout.hit(x, y);
    }

    /// Handles pointer press; returns `true` if captured by the dialog.
    pub fn pointer_down(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &ShopSnapshot,
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
        self.pressed_item = match hit {
            Some(ShopHit::BuyButton | ShopHit::TryOnButton) => self
                .selected_item(snapshot)
                .map(|i| (i.id.clone(), i.currency, i.price)),
            Some(ShopHit::ItemCard(idx)) => snapshot
                .items
                .get(idx)
                .map(|i| (i.id.clone(), i.currency, i.price)),
            _ => None,
        };
        hit.is_some() || layout.dialog.contains(x, y)
    }

    /// Handles pointer release, emitting a [`ShopCommand`] when an enabled
    /// button is activated.
    pub fn pointer_up(
        &mut self,
        frame_size: [u32; 2],
        snapshot: &ShopSnapshot,
        x: i32,
        y: i32,
    ) -> Option<ShopCommand> {
        if !self.open {
            return None;
        }
        let pressed = self.pressed.take()?;
        let pressed_item = self.pressed_item.take();
        let layout = self.layout(frame_size, snapshot);
        let released = layout.hit(x, y);
        self.hovered = released;
        if released != Some(pressed) {
            return None;
        }
        match pressed {
            ShopHit::ItemCard(idx) => {
                if let Some(item) = snapshot.items.get(idx)
                    && pressed_item
                        .as_ref()
                        .is_some_and(|(id, _, _)| id == &item.id)
                {
                    self.selected_item_id = Some(item.id.clone());
                    self.follow_selection = false;
                }
                None
            }
            ShopHit::PreviousItem => self.choose(snapshot, false),
            ShopHit::NextItem => self.choose(snapshot, true),
            ShopHit::ScrollUp => {
                self.scroll(-1, frame_size, snapshot);
                None
            }
            ShopHit::ScrollDown => {
                self.scroll(1, frame_size, snapshot);
                None
            }
            ShopHit::RotateLeft => snapshot
                .preview_image
                .is_some()
                .then_some(ShopCommand::RotatePreview(-45)),
            ShopHit::RotateRight => snapshot
                .preview_image
                .is_some()
                .then_some(ShopCommand::RotatePreview(45)),
            ShopHit::TryOnButton => {
                let (expected_id, _, _) = pressed_item?;
                if !self.can_try_on_selected(snapshot) {
                    return None;
                }
                self.selected_item(snapshot)
                    .filter(|item| item.id == expected_id)
                    .map(|item| ShopCommand::TryOn {
                        item_id: item.id.clone(),
                    })
            }
            ShopHit::BuyButton => {
                if snapshot.choice.is_some() && self.chosen_item(snapshot).is_none() {
                    return Some(ShopCommand::NoSelection);
                }
                let (expected_id, expected_currency, expected_price) = pressed_item?;
                if !self.can_buy_selected(snapshot) {
                    return None;
                }
                self.selected_item(snapshot)
                    .filter(|item| {
                        item.id == expected_id
                            && item.currency == expected_currency
                            && item.price == expected_price
                    })
                    .map(|item| ShopCommand::Buy {
                        item_id: item.id.clone(),
                        currency: item.currency,
                        price: item.price,
                    })
            }
            ShopHit::CancelButton => {
                self.close();
                Some(ShopCommand::Cancel)
            }
        }
    }

    /// A choice dialog's arrow: the next or previous item, wrapping at
    /// either end as native does (`0x40d229` past the last goes to the
    /// first, `0x40d360` before the first goes to the last).
    ///
    /// With [`ShopSnapshot::body_shapes`], items of another body shape than
    /// the shown one are passed over (`0x40d27a..0x40d28c`); an arrow that
    /// comes back to the shown item without finding one leaves the view as
    /// it is (`0x40d222`). With no item shown, next goes to the first item
    /// and previous to the last, unfiltered (native reads a shape from
    /// before its list there).
    ///
    /// The item moved to is always tried on, so the view (or its fallback)
    /// follows the selection even when no picture can be rendered. None
    /// when there is nothing to move to.
    pub fn choose(&mut self, snapshot: &ShopSnapshot, next: bool) -> Option<ShopCommand> {
        let count = snapshot.items.len();
        if count == 0 {
            return None;
        }
        let current = self
            .chosen_item(snapshot)
            .and_then(|item| snapshot.items.iter().position(|i| i.id == item.id));
        let step = |idx: usize| {
            if next {
                (idx + 1) % count
            } else {
                (idx + count - 1) % count
            }
        };
        let to = match current {
            Some(idx) => {
                let shapes = (snapshot.body_shapes.len() == count).then_some(&snapshot.body_shapes);
                let mut to = step(idx);
                while to != idx && shapes.is_some_and(|shapes| shapes[to] != shapes[idx]) {
                    to = step(to);
                }
                if to == idx {
                    return None;
                }
                to
            }
            None if next => 0,
            None => count - 1,
        };
        let item = &snapshot.items[to];
        self.select_item(item.id.clone());
        Some(ShopCommand::TryOn {
            item_id: item.id.clone(),
        })
    }

    fn button_state(&self, target: ShopHit, enabled: bool) -> ButtonState {
        if !enabled {
            ButtonState::Disabled
        } else if self.pressed == Some(target) && self.hovered == Some(target) {
            ButtonState::Selected
        } else if self.hovered == Some(target) {
            ButtonState::Hover
        } else {
            ButtonState::Normal
        }
    }

    /// Draws the shop / wardrobe / pet adoption / Magic Town vendor picker.
    pub fn draw(
        &self,
        frame: &mut Frame,
        snapshot: &ShopSnapshot,
        strings: &ShopStrings,
        art: &Art,
    ) {
        if !self.open {
            return;
        }
        let layout = self.layout([frame.width(), frame.height()], snapshot);
        draw_dialog_frame(frame, art, layout.dialog);
        if let Some(choice) = snapshot.choice {
            self.draw_choice(frame, snapshot, strings, art, &layout, choice);
            return;
        }

        let title_font = art.font(12).or_else(|| art.font(10));
        let body_font = art.font(10).or_else(|| art.font(12));
        let small_font = art.font(9).or_else(|| art.font(10));

        // Header title.
        if let Some(font) = title_font {
            let base_title = snapshot
                .title_override
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| strings.title_for(snapshot.kind));
            let title = match snapshot
                .shopper_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(name) => format!("{base_title} — {name}"),
                None => base_title.to_owned(),
            };
            draw_clipped_text(
                frame,
                font,
                &title,
                [layout.title_bar.x, layout.title_bar.y + 1],
                layout.title_bar.width,
                TEXT_HOVER,
            );
        }

        // Wallet balance bar (Simoleons & MagiCoins).
        if !snapshot.hide_wallet_and_prices
            && layout.wallet_bar.height > 0
            && let Some(font) = small_font
        {
            let sim_str = format!(
                "Funds: {}",
                CurrencyKind::Simoleons.format_price(snapshot.wallet.simoleons)
            );
            let mc_str = format!(
                "MagiCoins: {}",
                CurrencyKind::MagiCoins.format_price(snapshot.wallet.magic_coins)
            );
            let show_magic = snapshot.kind == ShopKind::MagicVendor
                || snapshot.wallet.magic_coins > 0
                || snapshot
                    .items
                    .iter()
                    .any(|i| i.currency == CurrencyKind::MagiCoins);
            let wallet_text = if show_magic {
                format!("{sim_str}   |   {mc_str}")
            } else {
                sim_str
            };
            draw_clipped_text(
                frame,
                font,
                &wallet_text,
                [layout.wallet_bar.x, layout.wallet_bar.y + 2],
                layout.wallet_bar.width,
                if snapshot.kind == ShopKind::MagicVendor {
                    TEXT_MAGIC
                } else {
                    TEXT_OK
                },
            );
        }

        // Scrollable item card list.
        frame.fill(layout.list_area, LIST_BG);
        draw_outline(frame, layout.list_area, LIST_BORDER);

        let selected_idx = self.selected_index(snapshot);
        if snapshot.items.is_empty() {
            if let Some(font) = body_font {
                draw_clipped_text(
                    frame,
                    font,
                    "No items available.",
                    [layout.list_area.x + 10, layout.list_area.y + 10],
                    (layout.list_area.width - 20).max(20),
                    TEXT_NORMAL,
                );
            }
        } else {
            for &(idx, card_rect) in &layout.visible_cards {
                let Some(item) = snapshot.items.get(idx) else {
                    continue;
                };
                let is_selected = selected_idx == Some(idx);
                let is_hovered = self.hovered == Some(ShopHit::ItemCard(idx));
                let is_tried = snapshot
                    .tried_on_item_id
                    .as_deref()
                    .is_some_and(|id| id == item.id);
                let affordable = snapshot.wallet.can_afford(item.price, item.currency);

                let bg = if is_selected {
                    CARD_SELECTED_BG
                } else if is_hovered {
                    CARD_HOVER_BG
                } else {
                    CARD_BG
                };
                frame.fill(card_rect, bg);
                draw_outline(
                    frame,
                    card_rect,
                    if is_selected {
                        TEXT_SELECTED
                    } else {
                        LIST_BORDER
                    },
                );

                // Icon / portrait slot (42x42 centered vertically).
                let icon_size = (card_rect.height - 10).clamp(24, 42);
                let icon_rect = Rect::new(
                    card_rect.x + 6,
                    card_rect.y + (card_rect.height - icon_size) / 2,
                    icon_size,
                    icon_size,
                );
                frame.fill(icon_rect, ICON_SLOT_BG);
                draw_outline(frame, icon_rect, LIST_BORDER);
                if let Some(icon) = item.icon.as_ref() {
                    draw_aspect_fit(
                        frame,
                        icon,
                        Rect::new(
                            icon_rect.x + 1,
                            icon_rect.y + 1,
                            (icon_rect.width - 2).max(1),
                            (icon_rect.height - 2).max(1),
                        ),
                    );
                }

                // Price label on top-right (hidden for non-monetary pickers).
                let text_x = icon_rect.x + icon_rect.width + 8;
                let desc_w = (card_rect.x + card_rect.width - text_x - 8).max(20);
                let name_w = if snapshot.hide_wallet_and_prices {
                    desc_w
                } else {
                    let price_str = item.currency.format_price(item.price);
                    let price_w = body_font.map_or(56, |f| f.width(&price_str).max(40)) + 8;
                    let price_x = (card_rect.x + card_rect.width - price_w - 4)
                        .max(icon_rect.x + icon_rect.width + 8);
                    let price_color = if !item.available || item.price < 0 {
                        TEXT_DISABLED
                    } else if item.owned {
                        TEXT_NORMAL
                    } else if !affordable {
                        TEXT_ERROR
                    } else if item.currency == CurrencyKind::MagiCoins {
                        TEXT_MAGIC
                    } else {
                        TEXT_OK
                    };
                    if let Some(font) = body_font {
                        draw_clipped_text(
                            frame,
                            font,
                            &price_str,
                            [price_x, card_rect.y + 5],
                            (card_rect.x + card_rect.width - price_x - 4).max(16),
                            price_color,
                        );
                    }
                    (price_x - text_x - 6).max(20)
                };
                if let Some(font) = body_font {
                    let mut name_text = item.name.clone();
                    if is_tried {
                        name_text.push_str(" [Preview]");
                    } else if item.owned {
                        name_text.push_str(" [Owned]");
                    }
                    draw_clipped_text(
                        frame,
                        font,
                        &name_text,
                        [text_x, card_rect.y + 5],
                        name_w,
                        if is_selected {
                            TEXT_SELECTED
                        } else if item.available {
                            TEXT_HOVER
                        } else {
                            TEXT_DISABLED
                        },
                    );
                }
                if let Some(font) = small_font {
                    let desc_rect = Rect::new(
                        text_x,
                        card_rect.y + 21,
                        desc_w,
                        (card_rect.height - 25).max(12),
                    );
                    draw_wrapped_text(frame, font, &item.description, desc_rect, TEXT_NORMAL);
                }
            }
        }

        // Scroll buttons.
        let capacity = (layout.list_area.height / CARD_HEIGHT).max(1) as usize;
        let max_scroll = snapshot.items.len().saturating_sub(capacity);
        let eff_scroll = self.effective_scroll(snapshot, capacity);
        if let Some(up) = layout.scroll_up {
            draw_button(
                frame,
                art,
                up,
                "^",
                self.button_state(ShopHit::ScrollUp, eff_scroll > 0),
            );
        }
        if let Some(down) = layout.scroll_down {
            draw_button(
                frame,
                art,
                down,
                "v",
                self.button_state(ShopHit::ScrollDown, eff_scroll < max_scroll),
            );
        }

        // Optional right-hand 3D Sim Try-On preview pane.
        if let Some(pane) = layout.preview_pane {
            frame.fill(pane, DETAIL_BG);
            draw_outline(frame, pane, LIST_BORDER);

            let rot_top = layout
                .rotate_left_btn
                .map_or(pane.y + pane.height - 4, |r| r.y - 4);
            let label_h = if snapshot.preview_label.is_some() {
                small_font.map_or(14, |f| f.line_height + 4)
            } else {
                0
            };
            let img_bounds = Rect::new(
                pane.x + 4,
                pane.y + 4,
                (pane.width - 8).max(1),
                (rot_top - (pane.y + 4) - label_h).max(16),
            );
            frame.fill(img_bounds, ICON_SLOT_BG);
            draw_outline(frame, img_bounds, LIST_BORDER);

            if let Some(img) = snapshot.preview_image.as_ref() {
                draw_aspect_fit(
                    frame,
                    img,
                    Rect::new(
                        img_bounds.x + 2,
                        img_bounds.y + 2,
                        (img_bounds.width - 4).max(1),
                        (img_bounds.height - 4).max(1),
                    ),
                );
            } else if let Some(font) = small_font {
                let fallback = snapshot
                    .preview_label
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Preview N/A");
                let text_rect = Rect::new(
                    img_bounds.x + 6,
                    img_bounds.y + (img_bounds.height / 2 - font.line_height).max(4),
                    (img_bounds.width - 12).max(16),
                    (img_bounds.height - 8).max(12),
                );
                draw_wrapped_text(frame, font, fallback, text_rect, TEXT_WARNING);
            }

            if let Some(label) = snapshot
                .preview_label
                .as_deref()
                .filter(|s| !s.trim().is_empty() && snapshot.preview_image.is_some())
                && let Some(font) = small_font
                && label_h > 0
            {
                draw_clipped_text(
                    frame,
                    font,
                    label,
                    [pane.x + 6, img_bounds.y + img_bounds.height + 2],
                    (pane.width - 12).max(16),
                    TEXT_SELECTED,
                );
            }

            let can_rotate = snapshot.preview_image.is_some();
            if let Some(left) = layout.rotate_left_btn {
                draw_button(
                    frame,
                    art,
                    left,
                    "<",
                    self.button_state(ShopHit::RotateLeft, can_rotate),
                );
            }
            if let Some(right) = layout.rotate_right_btn {
                draw_button(
                    frame,
                    art,
                    right,
                    ">",
                    self.button_state(ShopHit::RotateRight, can_rotate),
                );
            }
        }

        // Detail / affordability status bar.
        draw_detail_bar(frame, art, layout.detail_bar);
        if let Some(font) = small_font {
            let (msg, color) = if let Some(hint) = self.selected_status_hint(snapshot) {
                (hint.to_owned(), TEXT_WARNING)
            } else if let Some(item) = self.selected_item(snapshot) {
                if snapshot.hide_wallet_and_prices {
                    (format!("Selected: {}", item.name), TEXT_NORMAL)
                } else {
                    (
                        format!(
                            "Selected: {} ({})",
                            item.name,
                            item.currency.format_price(item.price)
                        ),
                        TEXT_NORMAL,
                    )
                }
            } else {
                ("Select an item.".to_owned(), TEXT_NORMAL)
            };
            let ty =
                layout.detail_bar.y + ((layout.detail_bar.height - font.line_height) / 2).max(2);
            draw_clipped_text(
                frame,
                font,
                &msg,
                [layout.detail_bar.x + 8, ty],
                (layout.detail_bar.width - 16).max(20),
                color,
            );
        }

        // Footer buttons: Try On (when shown), Buy/Confirm, Cancel.
        if layout.try_on_btn.width > 0 && layout.try_on_btn.height > 0 {
            draw_button(
                frame,
                art,
                layout.try_on_btn,
                &strings.try_on,
                self.button_state(ShopHit::TryOnButton, self.can_try_on_selected(snapshot)),
            );
        }
        let buy_label = snapshot
            .buy_label_override
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| strings.buy_label_for(snapshot.kind));
        draw_button(
            frame,
            art,
            layout.buy_btn,
            buy_label,
            self.button_state(ShopHit::BuyButton, self.can_buy_selected(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.cancel_btn,
            &strings.cancel,
            self.button_state(ShopHit::CancelButton, true),
        );
    }
}

impl ShopPickerDialog {
    /// The native choice dialog: title, 3D view, arrows, OK and Cancel.
    fn draw_choice(
        &self,
        frame: &mut Frame,
        snapshot: &ShopSnapshot,
        strings: &ShopStrings,
        art: &Art,
        layout: &ShopLayout,
        choice: ChoiceDialog,
    ) {
        if let Some(font) = art.font(12).or_else(|| art.font(10)) {
            let title = strings.choice_title(choice);
            let width = font.width(title).min(layout.title_bar.width);
            draw_clipped_text(
                frame,
                font,
                title,
                [
                    layout.title_bar.x + (layout.title_bar.width - width) / 2,
                    layout.title_bar.y + 1,
                ],
                layout.title_bar.width,
                TEXT_HOVER,
            );
        }
        match (layout.preview_pane, snapshot.preview_image.as_ref()) {
            (Some(view), Some(image)) => {
                // Rendered at the view's size; a smaller picture sits centred.
                let x = view.x + (view.width - image.width as i32) / 2;
                let y = view.y + (view.height - image.height as i32) / 2;
                frame.draw_all(image, [x, y]);
            }
            (Some(view), None) => {
                // Not native: when the 3D view cannot render (a missing
                // skeleton or mesh), name the candidate so OK never accepts
                // an unseen choice.
                if let Some(font) = art.font(10).or_else(|| art.font(12)) {
                    let name = self.chosen_item(snapshot).map(|item| item.name.as_str());
                    let note = snapshot
                        .preview_label
                        .as_deref()
                        .filter(|label| Some(*label) != name);
                    let line_h = 14;
                    let mut y = view.y + view.height / 2 - line_h;
                    for line in name.into_iter().chain(note) {
                        let width = font.width(line).min(view.width);
                        draw_clipped_text(
                            frame,
                            font,
                            line,
                            [view.x + (view.width - width) / 2, y],
                            view.width,
                            TEXT_HOVER,
                        );
                        y += line_h;
                    }
                }
            }
            _ => {}
        }
        // Native arrows wrap, so both stay live while there is a choice.
        let movable = snapshot.items.len() > 1;
        for (rect, hit, id, enabled) in [
            (
                layout.previous_btn,
                ShopHit::PreviousItem,
                CLOTHING_PREVIOUS,
                movable,
            ),
            (layout.next_btn, ShopHit::NextItem, CLOTHING_NEXT, movable),
        ] {
            let Some(rect) = rect else {
                continue;
            };
            let state = self.button_state(hit, enabled);
            if let Some(cell) = art.cell(id, 4, state.cell()) {
                frame.draw_all(&cell, [rect.x, rect.y]);
            } else {
                draw_button(frame, art, rect, "", state);
            }
        }
        draw_button(
            frame,
            art,
            layout.buy_btn,
            &strings.ok,
            self.button_state(ShopHit::BuyButton, self.can_buy_selected(snapshot)),
        );
        draw_button(
            frame,
            art,
            layout.cancel_btn,
            &strings.cancel,
            self.button_state(ShopHit::CancelButton, true),
        );
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

fn draw_detail_bar(frame: &mut Frame, art: &Art, rect: Rect) {
    if let Some(tiler) = art.image(POPUP_TILER)
        && tiler.width >= 3
        && tiler.height >= 3
    {
        neighborhood::draw_nine_slice(frame, tiler, rect);
    } else {
        frame.fill(rect, DETAIL_BG);
        draw_outline(frame, rect, LIST_BORDER);
    }
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
        draw_outline(frame, rect, LIST_BORDER);
    }

    if let Some(font) = art.font(10).or_else(|| art.font(12)) {
        let avail_w = (rect.width - 8).max(8);
        let text_w = font.width(label).min(avail_w);
        let tx = rect.x + ((rect.width - text_w) / 2).max(4);
        let ty = rect.y + ((rect.height - font.line_height) / 2).max(2);
        draw_clipped_text(frame, font, label, [tx, ty], avail_w, state.text_color());
    }
}

fn draw_aspect_fit(frame: &mut Frame, image: &Image, bounds: Rect) {
    if image.width == 0 || image.height == 0 || bounds.width <= 0 || bounds.height <= 0 {
        return;
    }
    let iw = i64::from(image.width);
    let ih = i64::from(image.height);
    let bw = i64::from(bounds.width);
    let bh = i64::from(bounds.height);
    let (dw, dh) = if iw * bh >= ih * bw {
        (bw, ((ih * bw) / iw).max(1))
    } else {
        (((iw * bh) / ih).max(1), bh)
    };
    let dx = bounds.x + ((bounds.width - dw as i32) / 2).max(0);
    let dy = bounds.y + ((bounds.height - dh as i32) / 2).max(0);
    let src = Rect::new(
        0,
        0,
        i32::try_from(image.width).unwrap_or(i32::MAX),
        i32::try_from(image.height).unwrap_or(i32::MAX),
    );
    draw_scaled_region(frame, image, src, Rect::new(dx, dy, dw as i32, dh as i32));
}

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
