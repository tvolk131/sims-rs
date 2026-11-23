//! Real graphical destination map picker for VM special lot-picker dialogs
//! (`Dialog` kinds `5, 7, 8, 11, 13`: Downtown, Vacation Island, Old Town,
//! Studio Town, and Magic Town).
//!
//! Renders the authentic `sims_render::neighborhood::NeighborhoodMap` backdrop,
//! lot thumbnails, 300 ms hover roof cross-fades, water waves, community filter
//! highlights, native top bar (`TravelChrome` in `TravelBarMode::Visit` or
//! Old Town's return banner), and native House Info hover cards — replacing the
//! placeholder `ShopPickerDialog` list.

use crate::game::SpecialLotEntry;
use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_render::Canvas;
use sims_render::neighborhood::{
    CommunityFilter, HoverFadeState, NeighborhoodKind, NeighborhoodMap, RenderOptions,
};
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::neighborhood::{
    ExpansionFlags, LotInfo, LotKind, NeighborhoodMode, NeighborhoodState, NeighborhoodStrings,
    draw_hovered_lot_popup,
};
use sims_ui::travel::{
    BAR_HEIGHT as TRAVEL_BAR_HEIGHT, Destination, LotFilter, NATIVE_HEIGHT as TRAVEL_NATIVE_HEIGHT,
    NATIVE_WIDTH as TRAVEL_NATIVE_WIDTH, TravelBarMode, TravelChrome, TravelCommand, TravelLayout,
    TravelState, TravelStrings, art_id as travel_art_id,
};
use sims_vm::SpecialPanelRequest;
use std::collections::{BTreeMap, BTreeSet};

/// Water wave animation cadence (matching `NeighborhoodShell::WATER_STEP_US`).
const WATER_STEP_US: u64 = 250_000;

/// Cached UI artwork and string tables reused across destination picker opens.
#[derive(Debug, Default)]
pub struct DestinationPickerAssets {
    pub travel_art: Art,
    pub travel_strings: TravelStrings,
    pub chrome_art: Art,
    pub chrome_strings: NeighborhoodStrings,
}

impl DestinationPickerAssets {
    /// Loads native destination bar, filter, and neighborhood hover-card art
    /// and string tables from `install` (gracefully tolerating missing
    /// expansion assets).
    pub fn load(install: &Install) -> Self {
        let travel_strings = TravelStrings::load(install, US_ENGLISH).unwrap_or_default();
        let chrome_strings = NeighborhoodStrings::load(install, US_ENGLISH).unwrap_or_default();
        let (travel_art, chrome_art) = match UiGraphics::open(install) {
            Ok(ui) => {
                let travel_art = sims_ui::travel::load_art(&ui, US_ENGLISH)
                    .or_else(|_| {
                        Art::load_with_optional(&ui, US_ENGLISH, &[], &sims_ui::travel::RESOURCES)
                    })
                    .unwrap_or_default();
                let chrome_art = sims_ui::neighborhood::load_art(&ui, US_ENGLISH)
                    .or_else(|_| {
                        Art::load_with_optional(
                            &ui,
                            US_ENGLISH,
                            &[],
                            &sims_ui::neighborhood::RESOURCES,
                        )
                    })
                    .unwrap_or_default();
                (travel_art, chrome_art)
            }
            Err(_) => (Art::default(), Art::default()),
        };
        Self {
            travel_art,
            travel_strings,
            chrome_art,
            chrome_strings,
        }
    }
}

/// Outcome of a user input on [`DestinationMapPicker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationPickerAction {
    /// The player clicked a visitable destination lot on the graphical map.
    SelectLot(u16),
    /// The player clicked Return / Cancel or pressed Escape.
    Cancel,
    /// The player clicked a non-visitable lot (e.g., a Magic Town or Old Town
    /// residential house) and should receive error feedback while staying on
    /// the map.
    InvalidLot,
    /// A completed interactive control action (e.g. toggling a filter chip)
    /// that should play a confirmation click and redraw.
    Clicked,
    /// Visual-only update (e.g. aborted release off a control/lot) that should
    /// redraw silently without playing a confirmation click.
    Redraw,
}

/// Returns the `(NeighborhoodKind, Option<Destination>)` corresponding to a
/// destination lot-picker [`SpecialPanelRequest`], or `None` for non-lot
/// special panels.
pub fn destination_kind_for_request(
    request: &SpecialPanelRequest,
) -> Option<(NeighborhoodKind, Option<Destination>)> {
    match request {
        SpecialPanelRequest::DowntownLotPicker => {
            Some((NeighborhoodKind::Downtown, Some(Destination::Downtown)))
        }
        SpecialPanelRequest::VacationLotPicker { .. } => Some((
            NeighborhoodKind::Vacation,
            Some(Destination::VacationIsland),
        )),
        SpecialPanelRequest::OldTownLotPicker { unleashed } => Some((
            if *unleashed {
                NeighborhoodKind::Unleashed
            } else {
                NeighborhoodKind::Classic
            },
            None,
        )),
        SpecialPanelRequest::StudioTownLotPicker { .. } => {
            Some((NeighborhoodKind::StudioTown, Some(Destination::StudioTown)))
        }
        SpecialPanelRequest::MagicTownLotPicker { .. } => {
            Some((NeighborhoodKind::MagicTown, Some(Destination::MagicTown)))
        }
        _ => None,
    }
}

/// Whether `lot_id` (with `zoning` from `LotZoning.iff` / `StoredNeighborhood`)
/// is a visitable destination target for `request`.
pub fn is_visitable_lot_for_request(
    request: &SpecialPanelRequest,
    lot_id: u16,
    zoning: Option<i16>,
) -> bool {
    match request {
        SpecialPanelRequest::DowntownLotPicker => (21..=30).contains(&lot_id),
        SpecialPanelRequest::VacationLotPicker { .. } => (40..=48).contains(&lot_id),
        SpecialPanelRequest::OldTownLotPicker { .. } => {
            if (21..=30).contains(&lot_id)
                || (40..=48).contains(&lot_id)
                || (81..=89).contains(&lot_id)
                || (90..=98).contains(&lot_id)
            {
                return false;
            }
            match zoning {
                Some(z) => z == 2,
                None => (58..=68).contains(&lot_id),
            }
        }
        SpecialPanelRequest::StudioTownLotPicker { .. } => (81..=89).contains(&lot_id),
        SpecialPanelRequest::MagicTownLotPicker { .. } => (93..=98).contains(&lot_id),
        _ => false,
    }
}

/// Interactive graphical destination map picker presented over an active lot
/// when the VM pauses on `Dialog` kinds `5, 7, 8, 11, 13`.
///
/// Always constructs a visible, cancellable picker even if the expansion map
/// artwork fails to load on a partial install, so a paused VM can never strand.
pub struct DestinationMapPicker {
    request: SpecialPanelRequest,
    kind: NeighborhoodKind,
    slot: u8,
    map: Option<NeighborhoodMap>,
    map_error: Option<String>,
    visitable_lots: BTreeSet<u16>,
    /// Valid visitable lots from the store-backed catalog whose thumbnail is
    /// missing from `map` (or all visitable lots when `map` failed to load),
    /// rendered as clickable fallback chips so no valid saved lot is omitted.
    fallback_lots: Vec<u16>,
    entries: BTreeMap<u16, SpecialLotEntry>,
    travel_chrome: TravelChrome,
    travel_state: Option<TravelState>,
    hovered_lot_id: Option<u16>,
    pressed_lot_id: Option<u16>,
    hover_fade: HoverFadeState,
    hover_fade_was_animating: bool,
    last_clock_us: u64,
    water_frame: usize,
    pointer: Option<[i32; 2]>,
    back_hovered: bool,
    back_pressed: bool,
    simoleons: i32,
}

impl DestinationMapPicker {
    /// Loads the real graphical [`NeighborhoodMap`] for `request` (in
    /// neighborhood `slot`, falling back to `TemplateUserData` when `UserData*`
    /// is absent) and indexes all store-backed visitable destination lots.
    ///
    /// If the map artwork fails to load (e.g., partial install lacking an
    /// expansion backdrop), still returns a visible, cancellable picker so the
    /// paused VM never strands.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        install: &Install,
        slot: u8,
        request: SpecialPanelRequest,
        lot_entries: Vec<SpecialLotEntry>,
        simoleons: i32,
        // The native destination bar shows no MagicCoin or fame readout
        // (`docs/decisions/20261123-travel-chrome-native-audit.md`).
        _magic_coins: i32,
        _studio_fame_half_stars: u8,
    ) -> Result<Self, String> {
        let Some((kind, destination)) = destination_kind_for_request(&request) else {
            return Err(format!(
                "special panel kind {} is not a destination lot picker",
                request.kind()
            ));
        };
        let slot = slot.clamp(1, 8);
        let (map, map_error) = match load_destination_map(install, slot, kind) {
            Ok(loaded) => (Some(loaded), None),
            Err(err) => (None, Some(err)),
        };

        let mut entries = BTreeMap::new();
        for entry in lot_entries {
            entries.insert(entry.lot_id, entry);
        }

        // Determine visitable lot eligibility from the authoritative
        // store-backed catalog when available so lots missing from the active
        // save are never offered and valid saved lots without thumbnails are
        // never omitted. Fall back to map lots only when no catalog was supplied.
        let mut visitable_lots = BTreeSet::new();
        if !entries.is_empty() {
            for entry in entries.values() {
                if is_visitable_lot_for_request(&request, entry.lot_id, Some(entry.zoning)) {
                    visitable_lots.insert(entry.lot_id);
                }
            }
        } else if let Some(loaded) = &map {
            for lot in &loaded.lots {
                if is_visitable_lot_for_request(&request, lot.id, None) {
                    visitable_lots.insert(lot.id);
                }
            }
        }

        let fallback_lots: Vec<u16> = visitable_lots
            .iter()
            .copied()
            .filter(|&id| map.as_ref().is_none_or(|m| m.lot(id).is_none()))
            .collect();

        // Picker filters with no visitable lot on this map draw disabled, as
        // the 1.91 oracle shows for Gardening on Magic Town.
        let unavailable_filters: Vec<LotFilter> = destination
            .map(|dest| {
                dest.picker_filters()
                    .iter()
                    .copied()
                    .filter(|&f| {
                        let bit = community_filter_for_lot_filter(f).bit();
                        map.as_ref().is_some_and(|m| {
                            !m.lots.iter().any(|lot| {
                                visitable_lots.contains(&lot.id) && lot.filter_mask & bit != 0
                            })
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let travel_state = destination.map(|dest| TravelState {
            destination: dest,
            mode: TravelBarMode::Visit,
            expansions: ExpansionFlags(install.expansions.0),
            has_importable_lot: false,
            // The 1.91 Magic Hole picker opens with the Magic filter pressed
            // (oracle `flow-magic-hole-picker.png`).
            active_filter: (dest == Destination::MagicTown
                && !unavailable_filters.contains(&LotFilter::Magic))
            .then_some(LotFilter::Magic),
            unavailable_filters,
        });

        Ok(Self {
            request,
            kind,
            slot,
            map,
            map_error,
            visitable_lots,
            fallback_lots,
            entries,
            travel_chrome: TravelChrome::new(),
            travel_state,
            hovered_lot_id: None,
            pressed_lot_id: None,
            hover_fade: HoverFadeState::default(),
            hover_fade_was_animating: false,
            last_clock_us: 0,
            water_frame: 0,
            pointer: None,
            back_hovered: false,
            back_pressed: false,
            simoleons,
        })
    }

    pub fn request(&self) -> &SpecialPanelRequest {
        &self.request
    }

    pub fn kind(&self) -> NeighborhoodKind {
        self.kind
    }

    pub fn map(&self) -> Option<&NeighborhoodMap> {
        self.map.as_ref()
    }

    pub fn map_error(&self) -> Option<&str> {
        self.map_error.as_deref()
    }

    pub fn visitable_lots(&self) -> &BTreeSet<u16> {
        &self.visitable_lots
    }

    pub fn fallback_lots(&self) -> &[u16] {
        &self.fallback_lots
    }

    pub fn hovered_lot(&self) -> Option<u16> {
        self.hovered_lot_id
    }

    /// Finds a clickable screen point on `lot_id` inside `size` (below the
    /// top bar / filter strip) using the exact same [`hit_test_lot`] path and
    /// active fade clock used by live input, or the fallback chip center if
    /// `lot_id` has no map thumbnail.
    pub fn hit_point_for_lot(&self, size: [i32; 2], lot_id: u16) -> Option<[i32; 2]> {
        let viewport = [size[0].max(1) as u32, size[1].max(1) as u32];
        let clock_us = self.last_clock_us;
        for (id, rect) in self.fallback_chip_rects(viewport) {
            if id == lot_id {
                let pt = [rect.x + rect.width / 2, rect.y + rect.height / 2];
                if !self.over_chrome(viewport, pt)
                    && self.hit_test_lot(viewport, pt, clock_us) == Some(lot_id)
                {
                    return Some(pt);
                }
            }
        }
        let map = self.map.as_ref()?;
        let lot = map.lot(lot_id)?;
        let [left, top, right, bottom] = map.lot_screen_rect(lot, viewport);
        let min_y = self.chrome_bottom_y(viewport);
        let y_start = top.max(min_y + 1);
        let y_end = bottom.min(size[1]);
        let x_start = left.max(0);
        let x_end = right.min(size[0]);
        for y in (y_start..y_end).step_by(2) {
            for x in (x_start..x_end).step_by(2) {
                if !self.over_chrome(viewport, [x, y])
                    && self.hit_test_lot(viewport, [x, y], clock_us) == Some(lot_id)
                {
                    return Some([x, y]);
                }
            }
        }
        None
    }

    /// Screen rectangle of the Return / Cancel button in the top bar for `size`.
    pub fn cancel_button_rect(&self, size: [i32; 2]) -> Option<Rect> {
        let viewport = [size[0].max(1) as u32, size[1].max(1) as u32];
        if let Some(state) = &self.travel_state {
            let layout = TravelLayout::compute(viewport[0], viewport[1], state);
            layout
                .controls
                .iter()
                .find(|c| c.control == sims_ui::travel::TravelControl::ReturnHome)
                .map(|c| c.rect)
        } else {
            Some(old_town_bar_layout(viewport).1)
        }
    }

    /// Advances animation timers (300 ms hover roof cross-fade & water waves)
    /// and returns whether the map needs a redraw.
    pub fn tick(&mut self, clock_us: u64) -> bool {
        self.last_clock_us = clock_us;
        let mut redraw = false;
        let now_ms = clock_us / 1_000;
        let animating = self.hover_fade.is_animating(now_ms);
        if animating || self.hover_fade_was_animating {
            redraw = true;
        }
        self.hover_fade_was_animating = animating;
        self.hover_fade.prune(now_ms);

        let next_wave = (clock_us / WATER_STEP_US) as usize;
        if next_wave != self.water_frame {
            self.water_frame = next_wave;
            let has_waves = self
                .map
                .as_ref()
                .is_some_and(|m| m.waves.iter().any(Option::is_some));
            let has_filter = self
                .travel_state
                .as_ref()
                .is_some_and(|s| s.active_filter.is_some());
            redraw |= has_waves || has_filter;
        }
        redraw
    }

    /// Next animation wakeup deadline in microseconds, if any animation is running.
    pub fn next_wake_us(&self, clock_us: u64, anim_interval_us: u64) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut consider = |us: u64| {
            best = Some(best.map_or(us, |prev| prev.min(us)));
        };
        let now_ms = clock_us / 1_000;
        if self.hover_fade.is_animating(now_ms) || self.hover_fade_was_animating {
            consider(anim_interval_us.max(1_000));
        }
        let has_waves = self
            .map
            .as_ref()
            .is_some_and(|m| m.waves.iter().any(Option::is_some));
        let has_filter = self
            .travel_state
            .as_ref()
            .is_some_and(|s| s.active_filter.is_some());
        if has_waves || has_filter {
            let rem = WATER_STEP_US - (clock_us % WATER_STEP_US);
            consider(rem.max(1_000));
        }
        best
    }

    /// Handles pointer motion over the destination map. Returns `true` if the
    /// rendered frame changed.
    pub fn hover(&mut self, size: [i32; 2], point: [i32; 2], clock_us: u64) -> bool {
        self.last_clock_us = clock_us;
        self.pointer = Some(point);
        let viewport = [size[0].max(1) as u32, size[1].max(1) as u32];
        let mut changed = false;

        let over_chrome = if let Some(state) = &self.travel_state {
            let layout = TravelLayout::compute(viewport[0], viewport[1], state);
            let prev = self.travel_chrome.hovered();
            self.travel_chrome.pointer_move(point[0], point[1], &layout);
            changed |= self.travel_chrome.hovered() != prev;
            layout.blocks_map(point[0], point[1])
        } else {
            let (bar_rect, back_rect) = old_town_bar_layout(viewport);
            let next_back = back_rect.contains(point[0], point[1]);
            changed |= next_back != self.back_hovered;
            self.back_hovered = next_back;
            bar_rect.contains(point[0], point[1])
        };

        let hit = if over_chrome {
            None
        } else {
            self.hit_test_lot(viewport, point, clock_us)
                .filter(|id| self.visitable_lots.contains(id))
        };
        if hit != self.hovered_lot_id {
            let prev = self.hovered_lot_id;
            self.hovered_lot_id = hit;
            self.update_hover_fade(prev, hit, clock_us);
            changed = true;
        }
        changed
    }

    /// Clears transient hover/press state across both `TravelChrome`, Old Town
    /// return bar, and lot capture when the pointer leaves or releases outside.
    pub fn leave(&mut self, clock_us: u64) -> bool {
        self.last_clock_us = clock_us;
        self.pointer = None;
        let mut changed = self.travel_chrome.clear_pointer();
        if self.back_hovered || self.back_pressed {
            self.back_hovered = false;
            self.back_pressed = false;
            changed = true;
        }
        if self.pressed_lot_id.take().is_some() {
            changed = true;
        }
        if let Some(prev) = self.hovered_lot_id.take() {
            self.update_hover_fade(Some(prev), None, clock_us);
            changed = true;
        }
        changed
    }

    /// Handles a left mouse press on the destination map. Returns `true` if the
    /// rendered frame changed.
    pub fn press(&mut self, size: [i32; 2], point: [i32; 2], clock_us: u64) -> bool {
        self.last_clock_us = clock_us;
        self.pointer = Some(point);
        let viewport = [size[0].max(1) as u32, size[1].max(1) as u32];
        if let Some(state) = &self.travel_state {
            let layout = TravelLayout::compute(viewport[0], viewport[1], state);
            if self.travel_chrome.pointer_down(point[0], point[1], &layout) {
                self.pressed_lot_id = None;
                return true;
            }
            if layout.blocks_map(point[0], point[1]) {
                self.pressed_lot_id = None;
                return false;
            }
        } else {
            let (bar_rect, back_rect) = old_town_bar_layout(viewport);
            if back_rect.contains(point[0], point[1]) {
                self.back_pressed = true;
                self.back_hovered = true;
                self.pressed_lot_id = None;
                return true;
            }
            self.back_pressed = false;
            if bar_rect.contains(point[0], point[1]) {
                self.pressed_lot_id = None;
                return false;
            }
        }

        let raw_hit = self.hit_test_lot(viewport, point, clock_us);
        self.pressed_lot_id = raw_hit;
        let visitable_hit = raw_hit.filter(|id| self.visitable_lots.contains(id));
        if visitable_hit != self.hovered_lot_id {
            let prev = self.hovered_lot_id;
            self.hovered_lot_id = visitable_hit;
            self.update_hover_fade(prev, visitable_hit, clock_us);
        }
        raw_hit.is_some()
    }

    /// Handles a left mouse release on the destination map.
    pub fn release(
        &mut self,
        size: [i32; 2],
        point: [i32; 2],
        clock_us: u64,
    ) -> Option<DestinationPickerAction> {
        self.last_clock_us = clock_us;
        self.pointer = Some(point);
        let viewport = [size[0].max(1) as u32, size[1].max(1) as u32];

        if let Some(state) = &mut self.travel_state {
            let layout = TravelLayout::compute(viewport[0], viewport[1], state);
            let had_chrome_press = self.travel_chrome.pressed().is_some();
            if let Some(cmd) = self
                .travel_chrome
                .pointer_up(point[0], point[1], &layout, state)
            {
                self.pressed_lot_id = None;
                return match cmd {
                    TravelCommand::ReturnToNeighborhood => Some(DestinationPickerAction::Cancel),
                    TravelCommand::SetFilter(filter) => {
                        state.active_filter = filter;
                        Some(DestinationPickerAction::Clicked)
                    }
                    _ => Some(DestinationPickerAction::Redraw),
                };
            }
            if layout.blocks_map(point[0], point[1]) {
                let had_lot = self.pressed_lot_id.take().is_some();
                return (had_chrome_press || had_lot).then_some(DestinationPickerAction::Redraw);
            }
        } else {
            let (bar_rect, back_rect) = old_town_bar_layout(viewport);
            let was_pressed = std::mem::take(&mut self.back_pressed);
            self.back_hovered = back_rect.contains(point[0], point[1]);
            if was_pressed && self.back_hovered {
                self.pressed_lot_id = None;
                return Some(DestinationPickerAction::Cancel);
            }
            if bar_rect.contains(point[0], point[1]) {
                let had_lot = self.pressed_lot_id.take().is_some();
                return (was_pressed || had_lot).then_some(DestinationPickerAction::Redraw);
            }
        }

        let pressed = self.pressed_lot_id.take()?;
        let hit = self.hit_test_lot(viewport, point, clock_us);
        if hit == Some(pressed) {
            if self.visitable_lots.contains(&pressed) {
                Some(DestinationPickerAction::SelectLot(pressed))
            } else {
                Some(DestinationPickerAction::InvalidLot)
            }
        } else {
            Some(DestinationPickerAction::Redraw)
        }
    }

    /// Renders the full graphical destination map, top bar, filter strip,
    /// fallback lot chips (if any), and hovered lot info card into `canvas`.
    pub fn draw(&mut self, canvas: &mut Canvas, assets: &DestinationPickerAssets, clock_us: u64) {
        self.last_clock_us = clock_us;
        let w = canvas.width;
        let h = canvas.height;
        if w == 0 || h == 0 {
            return;
        }
        let viewport = [w as u32, h as u32];
        if clock_us > 0 {
            self.hover_fade.prune(clock_us / 1_000);
        }
        let opts = self.build_render_options(viewport, clock_us);
        if let Some(map) = &self.map {
            map.render_into(canvas, &opts);
        } else {
            canvas.clear([14, 24, 68]);
        }

        let packed = std::mem::take(&mut canvas.color);
        if let Ok(mut frame) = Frame::from_packed(viewport[0], viewport[1], packed) {
            if let Some(state) = &self.travel_state {
                self.travel_chrome.draw(
                    &mut frame,
                    state,
                    &assets.travel_art,
                    &assets.travel_strings,
                );
            } else {
                self.draw_old_town_bar(&mut frame, assets);
            }

            if self.map.is_none() {
                self.draw_missing_map_notice(&mut frame, assets, viewport);
            }
            if !self.fallback_lots.is_empty() {
                self.draw_fallback_lot_chips(&mut frame, assets, viewport);
            }

            if let Some(lot_id) = self.hovered_lot_id
                && let Some(info) = self.build_lot_info(lot_id, viewport)
            {
                let nb_state = NeighborhoodState {
                    neighborhood_number: self.slot,
                    mode: NeighborhoodMode::Visit,
                    expansions: ExpansionFlags::ALL,
                    has_importable_family: false,
                    show_credits_button: false,
                    hovered_lot: Some(info.clone()),
                };
                draw_hovered_lot_popup(
                    &mut frame,
                    &info,
                    &nb_state,
                    &assets.chrome_art,
                    &assets.chrome_strings,
                );
            }
            canvas.color = frame.into_packed();
        } else {
            canvas.color = vec![0u32; w * h];
        }
    }

    fn build_render_options(&self, viewport: [u32; 2], clock_us: u64) -> RenderOptions {
        let active_filter = self
            .travel_state
            .as_ref()
            .and_then(|s| s.active_filter.map(community_filter_for_lot_filter));
        let highlighted_lots = self.highlighted_lots();
        let mut opts = RenderOptions {
            viewport,
            hovered_lot: self.hovered_lot_id,
            hover_roofs_off: true,
            highlight_hovered: self.hovered_lot_id.is_some(),
            highlighted_lots,
            water_frame: Some(self.water_frame),
            active_filter,
            filter_frame: self.water_frame,
            show_occupancy_markers: false,
            ..RenderOptions::default()
        };
        if clock_us > 0 {
            let now_ms = clock_us / 1_000;
            self.hover_fade.apply_to(&mut opts, now_ms);
            if let Some(lot) = self.hovered_lot_id
                && self.hover_fade.hovered() != Some(lot)
            {
                opts.hovered_lot = Some(lot);
                opts.hover_roof_fade = 1.0;
                opts.lot_roof_fades.insert(lot, 1.0);
            }
        }
        opts
    }

    fn hit_test_lot(&self, viewport: [u32; 2], point: [i32; 2], clock_us: u64) -> Option<u16> {
        for (id, rect) in self.fallback_chip_rects(viewport) {
            if rect.contains(point[0], point[1]) {
                return Some(id);
            }
        }
        let map = self.map.as_ref()?;
        let opts = self.build_render_options(viewport, clock_us);
        map.hit_test_with_options(&opts, point)
    }

    fn highlighted_lots(&self) -> BTreeSet<u16> {
        if matches!(
            self.kind,
            NeighborhoodKind::Unleashed | NeighborhoodKind::Classic
        ) {
            // On Old Town / main neighborhood maps, highlight all visitable
            // community lots so they stand out cleanly from residential houses.
            return self.visitable_lots.clone();
        }
        BTreeSet::new()
    }

    fn build_lot_info(&self, lot_id: u16, viewport: [u32; 2]) -> Option<LotInfo> {
        let anchor = if let Some(rect) = self
            .fallback_chip_rects(viewport)
            .into_iter()
            .find_map(|(id, r)| (id == lot_id).then_some(r))
        {
            [rect.x + rect.width / 2, rect.y]
        } else {
            let map = self.map.as_ref()?;
            let lot = map.lot(lot_id)?;
            let [left, top, right, bottom] = map.lot_screen_rect(lot, viewport);
            [(left + right) / 2, (top + bottom) / 2]
        };
        let entry = self.entries.get(&lot_id);
        let house_name = entry
            .map(|e| e.name.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Lot {lot_id}"));
        let address = entry.and_then(|e| e.address.clone());
        let house_description = entry
            .map(|e| e.description.trim().to_owned())
            .unwrap_or_default();
        let kind = if self.kind == NeighborhoodKind::MagicTown {
            LotKind::MagicCommunity
        } else {
            LotKind::Community
        };
        Some(LotInfo {
            lot_number: lot_id,
            address,
            house_name,
            house_description,
            kind,
            has_house: true,
            house_cost: 0,
            magic_cost: 0,
            occupant: None,
            anchor,
        })
    }

    fn fallback_chip_rects(&self, viewport: [u32; 2]) -> Vec<(u16, Rect)> {
        if self.fallback_lots.is_empty() {
            return Vec::new();
        }
        let w = i32::try_from(viewport[0]).unwrap_or(i32::MAX).max(1);
        let h = i32::try_from(viewport[1]).unwrap_or(i32::MAX).max(1);
        let gap = 8;
        let pad = 16.min((w - 1) / 2);
        let chip_h = 30;
        let min_chip_w = 88;
        let max_chip_w = 160;
        let avail_w = (w - pad * 2).max(1);
        let max_cols = ((avail_w + gap) / (min_chip_w + gap)).max(1) as usize;
        let cols = self.fallback_lots.len().min(max_cols).max(1);
        let rows = self.fallback_lots.len().div_ceil(cols);
        let chip_w = (((avail_w - gap * (cols as i32 - 1)).max(cols as i32)) / cols as i32)
            .clamp(1, max_chip_w);
        let min_y = self.chrome_bottom_y(viewport) + 8;
        let total_h = rows as i32 * chip_h + (rows as i32 - 1) * gap;
        let start_y = (h - 14 - total_h).max(min_y);

        let mut rects = Vec::with_capacity(self.fallback_lots.len());
        for (row_idx, chunk) in self.fallback_lots.chunks(cols).enumerate() {
            let row_count = chunk.len() as i32;
            let row_w = row_count * chip_w + (row_count - 1) * gap;
            let row_x = ((w - row_w) / 2).max(pad);
            let y = start_y + row_idx as i32 * (chip_h + gap);
            for (col_idx, &lot_id) in chunk.iter().enumerate() {
                let x = row_x + col_idx as i32 * (chip_w + gap);
                let clamped_w = chip_w.min((w - pad - x).max(1));
                rects.push((lot_id, Rect::new(x, y, clamped_w, chip_h)));
            }
        }
        rects
    }

    fn draw_fallback_lot_chips(
        &self,
        frame: &mut Frame,
        assets: &DestinationPickerAssets,
        viewport: [u32; 2],
    ) {
        let font = assets
            .travel_art
            .font(10)
            .or_else(|| assets.chrome_art.font(10));
        for (lot_id, rect) in self.fallback_chip_rects(viewport) {
            let hovered = self.hovered_lot_id == Some(lot_id);
            let pressed = self.pressed_lot_id == Some(lot_id);
            let bg = if pressed {
                [66, 132, 218]
            } else if hovered {
                [44, 78, 148]
            } else {
                [24, 38, 86]
            };
            let border = if hovered {
                [255, 224, 112]
            } else {
                [96, 134, 210]
            };
            let text_color = [235, 242, 255];
            frame.fill(rect, bg);
            frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), border);
            frame.fill(
                Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
                border,
            );
            frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), border);
            frame.fill(
                Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
                border,
            );
            if let Some(f) = font {
                let label = self
                    .entries
                    .get(&lot_id)
                    .map(|e| e.name.trim())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Lot {lot_id}"));
                let tx = rect.x + (rect.width - f.width(&label)).max(4) / 2;
                let ty = rect.y + (rect.height - f.line_height) / 2;
                f.draw(frame, &label, [tx, ty], text_color);
            }
        }
    }

    fn draw_missing_map_notice(
        &self,
        frame: &mut Frame,
        assets: &DestinationPickerAssets,
        viewport: [u32; 2],
    ) {
        let w = i32::try_from(viewport[0]).unwrap_or(800);
        let h = i32::try_from(viewport[1]).unwrap_or(600);
        let card_w = 460.min(w - 32).max(200);
        let card_h = 104;
        let rect = Rect::new((w - card_w) / 2, (h - card_h) / 2, card_w, card_h);
        frame.fill(rect, [22, 32, 82]);
        let border = [110, 158, 242];
        frame.fill(Rect::new(rect.x, rect.y, rect.width, 2), border);
        frame.fill(
            Rect::new(rect.x, rect.y + rect.height - 2, rect.width, 2),
            border,
        );
        frame.fill(Rect::new(rect.x, rect.y, 2, rect.height), border);
        frame.fill(
            Rect::new(rect.x + rect.width - 2, rect.y, 2, rect.height),
            border,
        );
        if let Some(f) = assets
            .travel_art
            .font(12)
            .or_else(|| assets.chrome_art.font(12))
            .or_else(|| assets.travel_art.font(10))
            .or_else(|| assets.chrome_art.font(10))
        {
            let title = "Destination map unavailable";
            let tx = rect.x + (rect.width - f.width(title)) / 2;
            f.draw(frame, title, [tx, rect.y + 18], [255, 232, 136]);
            let hint = if self.fallback_lots.is_empty() {
                "Press Escape or click Back to go back to the lot."
            } else {
                "Choose a lot below, or press Escape / click Back to cancel."
            };
            let hx = rect.x + (rect.width - f.width(hint)).max(8) / 2;
            f.draw(frame, hint, [hx, rect.y + 52], [220, 234, 255]);
        }
    }

    fn draw_old_town_bar(&self, frame: &mut Frame, assets: &DestinationPickerAssets) {
        let viewport = [frame.width(), frame.height()];
        let (bar_rect, back_rect) = old_town_bar_layout(viewport);
        let compact = bar_rect.width < TRAVEL_NATIVE_WIDTH;
        let bar_bg = assets
            .travel_art
            .image(travel_art_id::NEIGHBORHOOD_BAR_BKG)
            .or_else(|| assets.travel_art.image(travel_art_id::DOWNTOWN_BAR_BKG));
        if !compact && let Some(img) = bar_bg {
            frame.draw_all(img, [bar_rect.x, bar_rect.y]);
        } else {
            frame.fill(bar_rect, [24, 34, 82]);
            frame.fill(
                Rect::new(
                    bar_rect.x,
                    bar_rect.y + bar_rect.height - 1,
                    bar_rect.width,
                    1,
                ),
                [76, 110, 184],
            );
        }

        // Return / Cancel button (`DOWNTOWN_RETURN` 4-cell strip).
        let cell = if self.back_pressed {
            1
        } else if self.back_hovered {
            2
        } else {
            0
        };
        if let Some(sprite) = assets
            .travel_art
            .cell(travel_art_id::DOWNTOWN_RETURN, 4, cell)
            && sprite.width as i32 <= back_rect.width + 6
            && sprite.height as i32 <= back_rect.height + 6
        {
            let sx = back_rect.x + (back_rect.width - sprite.width as i32) / 2;
            let sy = back_rect.y + (back_rect.height - sprite.height as i32) / 2;
            frame.draw_all(&sprite, [sx, sy]);
        } else {
            let bg = if self.back_pressed {
                [66, 132, 218]
            } else if self.back_hovered {
                [48, 86, 156]
            } else {
                [30, 44, 96]
            };
            frame.fill(back_rect, bg);
            if let Some(font) = assets
                .travel_art
                .font(10)
                .or_else(|| assets.chrome_art.font(10))
            {
                let label = "Back";
                let tx = back_rect.x + (back_rect.width - font.width(label)) / 2;
                let ty = back_rect.y + (back_rect.height - font.line_height) / 2;
                font.draw(frame, label, [tx, ty], [235, 242, 255]);
            }
        }

        let font = assets
            .travel_art
            .font(12)
            .or_else(|| assets.travel_art.font(10))
            .or_else(|| assets.chrome_art.font(12))
            .or_else(|| assets.chrome_art.font(10));
        if let Some(f) = font {
            let title = if self.kind == NeighborhoodKind::Classic {
                "Neighborhood"
            } else {
                "Old Town"
            };
            let tx = bar_rect.x + (bar_rect.width - f.width(title)) / 2;
            let ty = bar_rect.y + (bar_rect.height - f.line_height) / 2;
            f.draw(frame, title, [tx, ty], [255, 232, 136]);

            let funds = format!("${}", self.simoleons.max(0));
            let fx = bar_rect.x + bar_rect.width - f.width(&funds) - 16;
            if fx > tx + f.width(title) + 12 {
                f.draw(frame, &funds, [fx, ty], [220, 235, 255]);
            }
        }

        if self.back_hovered
            && let Some(tip_font) = assets
                .travel_art
                .font(10)
                .or_else(|| assets.chrome_art.font(10))
        {
            let tip = "Return to Lot (Esc)";
            let pad = 6;
            let tw = tip_font.width(tip) + pad * 2;
            let th = tip_font.line_height + 4;
            let tx = (back_rect.x + back_rect.width / 2 - tw / 2)
                .clamp(4, (frame.width() as i32 - tw - 4).max(4));
            let ty = (back_rect.y + back_rect.height + 4).min(frame.height() as i32 - th - 4);
            let rect = Rect::new(tx, ty, tw, th);
            frame.fill(rect, [255, 255, 230]);
            frame.fill(Rect::new(rect.x, rect.y, rect.width, 1), [0, 0, 0]);
            frame.fill(
                Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
                [0, 0, 0],
            );
            frame.fill(Rect::new(rect.x, rect.y, 1, rect.height), [0, 0, 0]);
            frame.fill(
                Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
                [0, 0, 0],
            );
            tip_font.draw(frame, tip, [rect.x + pad, rect.y + 2], [0, 0, 0]);
        }
    }

    fn update_hover_fade(&mut self, prev: Option<u16>, next: Option<u16>, clock_us: u64) {
        if prev == next {
            return;
        }
        let now_ms = clock_us / 1_000;
        if let Some(lot) = next {
            self.hover_fade.set_hovered(Some(lot), now_ms);
        } else {
            self.hover_fade.set_hovered(None, now_ms);
        }
    }

    fn chrome_bottom_y(&self, viewport: [u32; 2]) -> i32 {
        if let Some(state) = &self.travel_state {
            let layout = TravelLayout::compute(viewport[0], viewport[1], state);
            layout.bar_rect.y + layout.bar_rect.height
        } else {
            let (bar_rect, _) = old_town_bar_layout(viewport);
            bar_rect.y + bar_rect.height
        }
    }

    fn over_chrome(&self, viewport: [u32; 2], point: [i32; 2]) -> bool {
        if let Some(state) = &self.travel_state {
            TravelLayout::compute(viewport[0], viewport[1], state).blocks_map(point[0], point[1])
        } else {
            let (bar_rect, _) = old_town_bar_layout(viewport);
            bar_rect.contains(point[0], point[1])
        }
    }
}

fn old_town_bar_layout(viewport: [u32; 2]) -> (Rect, Rect) {
    let w = i32::try_from(viewport[0]).unwrap_or(i32::MAX).max(0);
    let h = i32::try_from(viewport[1]).unwrap_or(i32::MAX).max(0);
    let compact = w < TRAVEL_NATIVE_WIDTH;
    let bar_w = if compact { w } else { TRAVEL_NATIVE_WIDTH };
    let bar_x = if compact { 0 } else { (w - bar_w) / 2 };
    let oy = if !compact && h > TRAVEL_NATIVE_HEIGHT {
        (h - TRAVEL_NATIVE_HEIGHT) / 2
    } else {
        0
    };
    let bar_h = TRAVEL_BAR_HEIGHT.min(h - oy);
    let bar_rect = Rect::new(bar_x, oy, bar_w, bar_h);
    let back_x = bar_x + if compact { 8 } else { 78 };
    let back_w = if compact { 44 } else { 53 };
    let back_h = (bar_h - 6).max(1);
    let back_y = oy + (bar_h - back_h) / 2;
    let back_rect = Rect::new(back_x, back_y, back_w, back_h);
    (bar_rect, back_rect)
}

fn load_destination_map(
    install: &Install,
    slot: u8,
    kind: NeighborhoodKind,
) -> Result<NeighborhoodMap, String> {
    NeighborhoodMap::load(install, slot, kind, false)
        .or_else(|err| {
            let fallback_dir = install.file("TemplateUserData").map_err(|_| err)?;
            NeighborhoodMap::load_from_dir(install, &fallback_dir, slot, kind, false)
        })
        .map_err(|e| e.to_string())
}

fn community_filter_for_lot_filter(filter: LotFilter) -> CommunityFilter {
    match filter {
        LotFilter::Lodging => CommunityFilter::Lodging,
        LotFilter::Food => CommunityFilter::Food,
        LotFilter::Gardening => CommunityFilter::Gardening,
        LotFilter::Shopping => CommunityFilter::Shopping,
        LotFilter::DogCat => CommunityFilter::Pets,
        LotFilter::SmallAnimal => CommunityFilter::SmallAnimals,
        LotFilter::Recreation => CommunityFilter::Park,
        LotFilter::Spa => CommunityFilter::Spa,
        LotFilter::Movie => CommunityFilter::Movie,
        LotFilter::Model => CommunityFilter::Model,
        LotFilter::Music => CommunityFilter::Music,
        LotFilter::Magic => CommunityFilter::Magic,
        LotFilter::Rides => CommunityFilter::Rides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn old_town_unleashed_flag_selects_classic_vs_unleashed_and_missing_map_stays_cancellable() {
        assert_eq!(
            destination_kind_for_request(&SpecialPanelRequest::OldTownLotPicker {
                unleashed: false
            }),
            Some((NeighborhoodKind::Classic, None))
        );
        assert_eq!(
            destination_kind_for_request(&SpecialPanelRequest::OldTownLotPicker {
                unleashed: true
            }),
            Some((NeighborhoodKind::Unleashed, None))
        );

        // Even on an empty directory lacking Unleashed/Classic artwork, open()
        // returns a visible, cancellable picker with fallback chips (never stranding).
        let empty = Install {
            root: PathBuf::from("/nonexistent-sims-install-for-test"),
            layout: sims_content::Layout::Cd,
            expansions: sims_content::Expansions(0),
        };
        let mut picker = DestinationMapPicker::open(
            &empty,
            1,
            SpecialPanelRequest::OldTownLotPicker { unleashed: false },
            vec![SpecialLotEntry {
                lot_id: 58,
                name: "Old Town Square".into(),
                address: None,
                description: "Community lot".into(),
                zoning: 2,
            }],
            1000,
            0,
            0,
        )
        .expect("picker must construct cancellable fallback when map fails to load");
        assert!(picker.map().is_none());
        assert!(picker.map_error().is_some());
        assert_eq!(picker.kind(), NeighborhoodKind::Classic);
        assert_eq!(picker.fallback_lots(), &[58]);

        // Cancel button still works cleanly without map art.
        let cancel_rect = picker
            .cancel_button_rect([800, 600])
            .expect("cancel rect exists");
        let pt = [
            cancel_rect.x + cancel_rect.width / 2,
            cancel_rect.y + cancel_rect.height / 2,
        ];
        assert!(picker.press([800, 600], pt, 0));
        assert_eq!(
            picker.release([800, 600], pt, 0),
            Some(DestinationPickerAction::Cancel)
        );
    }

    #[test]
    fn fallback_chips_wrap_cleanly_within_800x600_for_eleven_old_town_lots() {
        let empty = Install {
            root: PathBuf::from("/nonexistent-sims-install-for-test"),
            layout: sims_content::Layout::Cd,
            expansions: sims_content::Expansions(0),
        };
        let entries: Vec<SpecialLotEntry> = (58..=68)
            .map(|lot_id| SpecialLotEntry {
                lot_id,
                name: format!("Old Town Lot {lot_id}"),
                address: None,
                description: "Community lot".into(),
                zoning: 2,
            })
            .collect();
        let mut picker = DestinationMapPicker::open(
            &empty,
            1,
            SpecialPanelRequest::OldTownLotPicker { unleashed: true },
            entries,
            1000,
            0,
            0,
        )
        .expect("fallback picker opens");
        assert_eq!(picker.fallback_lots().len(), 11);

        let rects = picker.fallback_chip_rects([800, 600]);
        assert_eq!(rects.len(), 11);
        for (lot_id, rect) in &rects {
            assert!(
                rect.x >= 0 && rect.x + rect.width <= 800,
                "lot {lot_id} chip {rect:?} must fit horizontally in 800px"
            );
            assert!(
                rect.y >= 0 && rect.y + rect.height <= 600,
                "lot {lot_id} chip {rect:?} must fit vertically in 600px"
            );
            let pt = picker
                .hit_point_for_lot([800, 600], *lot_id)
                .expect("every wrapped fallback chip must be clickable");
            assert!(picker.press([800, 600], pt, 0));
            assert_eq!(
                picker.release([800, 600], pt, 0),
                Some(DestinationPickerAction::SelectLot(*lot_id))
            );
        }
    }
}
