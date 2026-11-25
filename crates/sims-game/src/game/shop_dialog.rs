//! The shop dialog and the destination picker's drawing.

use super::*;

impl Game {
    /// Whether the travel / community-lot [`ShopPickerDialog`] is open.
    pub fn shop_open(&self) -> bool {
        self.shop_dialog.is_open()
    }

    /// Active [`ShopPickerDialog`] controller.
    pub fn shop_dialog(&self) -> &ShopPickerDialog {
        &self.shop_dialog
    }

    /// Active [`ShopSnapshot`], when the shop picker is open.
    pub fn shop_snapshot(&self) -> Option<&ShopSnapshot> {
        self.shop_snapshot.as_ref()
    }

    /// Default [`ShopKind`] matching the active lot's expansion / community zoning.
    ///
    /// Returns `None` on residential lots (including Magic Town residential lots
    /// 90–92) and during unlimited-budget community edit mode (`edit_mode` /
    /// `globals[32] != 0`) or when no live household (`globals[9]`) is active.
    pub fn shop_kind_for_lot(&self) -> Option<ShopKind> {
        let world = self.lot.world();
        let has_household = live_household(world);
        let edit_budget = world.globals.get(32).copied().unwrap_or(0) != 0;
        if self.edit_mode || edit_budget || !has_household || self.lot.residential() {
            return None;
        }
        let lot_id = self.lot.lot_id();
        match lot_id {
            21..=30 => Some(ShopKind::DowntownBoutique(WardrobeCategory::Formal)),
            40..=48 => Some(ShopKind::VacationKiosk),
            81..=89 => Some(ShopKind::StudioWardrobe),
            90..=98 => Some(ShopKind::MagicVendor),
            _ => Some(ShopKind::PetAdoption),
        }
    }

    /// Opens the community-lot [`ShopPickerDialog`] for `kind` with curated
    /// destination items and the active household's live Simoleons & MagiCoins wallet.
    pub fn open_shop(&mut self, kind: ShopKind) -> bool {
        let items = build_default_shop_items(kind, &self.purchased_shop_items);
        self.open_shop_with_items(kind, items)
    }

    /// Opens [`ShopPickerDialog`] with explicit `items` and the live household wallet.
    pub fn open_shop_with_items(&mut self, kind: ShopKind, mut items: Vec<ShopItem>) -> bool {
        let world = self.lot.world();
        let has_household = live_household(world);
        let edit_budget = world.globals.get(32).copied().unwrap_or(0) != 0;
        if self.edit_mode || edit_budget || !has_household {
            self.note = Some("Shop is only available during a live household visit.".into());
            return false;
        }
        self.ensure_shop_assets();
        for item in &mut items {
            if self.purchased_shop_items.contains(&item.id) {
                item.owned = true;
            }
        }
        let wallet = self.current_shop_wallet();
        let shopper_name = self.lot.world().selected_whereabouts().and_then(|_| {
            let sel = self.lot.world().globals.get(1).copied().unwrap_or(0) as u16;
            self.lot
                .world()
                .objects
                .get(&sel)
                .and_then(|o| self.hud.name(o.guid))
                .map(str::to_owned)
        });
        let preview_targets = self.build_boutique_preview_targets(kind, &items);
        self.shop_snapshot = Some(ShopSnapshot {
            kind,
            title_override: None,
            buy_label_override: None,
            shopper_name,
            wallet,
            items,
            tried_on_item_id: None,
            preview_image: None,
            preview_label: None,
            hide_wallet_and_prices: false,
            choice: None,
            body_shapes: Vec::new(),
        });
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        self.shop_preview_targets = preview_targets;
        self.shop_dialog.open();
        self.close_menu();
        true
    }

    /// Closes the open [`ShopPickerDialog`].
    pub fn close_shop(&mut self) {
        // The pointer last moved over the shop, not the lot: listing resumes
        // only on fresh pointer input, so closing never runs a lot check.
        // Unconditional: Escape and the Cancel button close the dialog
        // before the Cancel command reaches here.
        self.pointer = None;
        self.reached = None;
        self.listed = None;
        self.shop_dialog.close();
        self.shop_snapshot = None;
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        self.shop_preview_targets.clear();
    }

    /// Current horizontal rotation angle (in degrees) of the live 3D Shop Try-On preview.
    pub fn shop_preview_yaw_deg(&self) -> i16 {
        self.shop_preview_yaw_deg
    }

    /// Triggers Try-On for the currently selected item in [`ShopPickerDialog`].
    pub fn shop_try_on_selected(&mut self) -> Outcome {
        let Some(item_id) = self.shop_snapshot.as_ref().and_then(|snap| {
            self.shop_dialog
                .can_try_on_selected(snap)
                .then(|| self.shop_dialog.selected_item(snap).map(|i| i.id.clone()))
                .flatten()
        }) else {
            return Outcome::default();
        };
        self.execute_shop_command(ShopCommand::TryOn { item_id })
    }

    /// Rotates the live 3D Shop Try-On preview by `delta_deg` degrees.
    pub fn shop_rotate_preview(&mut self, delta_deg: i16) -> Outcome {
        if !self.shop_dialog.is_open()
            || self
                .shop_snapshot
                .as_ref()
                .is_none_or(|s| s.preview_image.is_none())
        {
            return Outcome::default();
        }
        self.execute_shop_command(ShopCommand::RotatePreview(delta_deg))
    }

    pub fn shop_select_prev(&mut self) -> bool {
        self.shop_step(false)
    }

    pub fn shop_select_next(&mut self) -> bool {
        self.shop_step(true)
    }

    /// Moves the shop selection one item. A choice dialog shows only its
    /// 3D view, so there the new item is tried on, as its arrows do.
    pub(super) fn shop_step(&mut self, next: bool) -> bool {
        let Some(snap) = self.shop_snapshot.as_ref() else {
            return false;
        };
        if snap.choice.is_none() {
            return if next {
                self.shop_dialog.select_next(snap)
            } else {
                self.shop_dialog.select_prev(snap)
            };
        }
        match self.shop_dialog.choose(snap, next) {
            Some(cmd) => {
                self.execute_shop_command(cmd);
                true
            }
            None => false,
        }
    }

    pub fn shop_wheel(&mut self, delta_y: f64) -> bool {
        let delta = if delta_y > 0.0 {
            -1
        } else if delta_y < 0.0 {
            1
        } else {
            return false;
        };
        let size = [
            self.view.size[0].max(1) as u32,
            self.view.size[1].max(1) as u32,
        ];
        self.shop_snapshot
            .as_ref()
            .is_some_and(|snap| self.shop_dialog.scroll(delta, size, snap))
    }

    pub fn shop_submit(&mut self) -> Outcome {
        let cmd = self
            .shop_snapshot
            .as_ref()
            .and_then(|snap| self.shop_dialog.submit(snap));
        match cmd {
            Some(cmd) => self.execute_shop_command(cmd),
            None => Outcome::default(),
        }
    }

    pub fn shop_escape(&mut self) -> Outcome {
        match self.shop_dialog.escape() {
            Some(cmd) => self.execute_shop_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Executes a [`ShopCommand`] emitted by [`ShopPickerDialog`], either
    /// answering an active VM special panel (`Dialog` kinds `5..=15`) or
    /// deducting Simoleons / MagiCoins atomically for an interactive community shop.
    pub fn execute_shop_command(&mut self, cmd: ShopCommand) -> Outcome {
        if let Some(active) = self.active_special_panel.clone() {
            return self.execute_special_panel_shop_command(active, cmd);
        }
        match cmd {
            // Community shops are lists, never choice dialogs.
            ShopCommand::Cancel | ShopCommand::NoSelection => {
                self.close_shop();
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::TryOn { item_id } => {
                let target = self.shop_preview_targets.get(&item_id).cloned();
                self.apply_shop_try_on(&item_id, target.as_ref());
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::RotatePreview(delta_deg) => {
                self.apply_shop_rotate_preview(delta_deg);
                self.hud.play(crate::hud::CLICK);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::Buy {
                item_id,
                currency,
                price,
            } => {
                if price < 0 {
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    return Outcome::redraw();
                }
                let wallet = self.current_shop_wallet();
                if !wallet.can_afford(price, currency) {
                    self.note = Some(format!(
                        "Insufficient funds for purchase ({}).",
                        currency.format_price(price)
                    ));
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    return Outcome::redraw();
                }
                let deduct_res = self.deduct_shop_currency(currency, price);
                match deduct_res {
                    Ok(()) => {
                        self.purchased_shop_items.insert(item_id.clone());
                        let next_wallet = self.current_shop_wallet();
                        let mut bought_name = item_id.clone();
                        if let Some(snap) = &mut self.shop_snapshot {
                            snap.wallet = next_wallet;
                            if let Some(item) = snap.items.iter_mut().find(|i| i.id == item_id) {
                                item.owned = true;
                                bought_name = item.name.clone();
                            }
                        }
                        self.note = Some(format!(
                            "Purchased {bought_name} for {}.",
                            currency.format_price(price)
                        ));
                        self.hud.play(crate::hud::MONEYBACK);
                    }
                    Err(e) => {
                        self.note = Some(format!("Purchase failed: {e}"));
                        self.hud.play(crate::hud::ERROR);
                    }
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
        }
    }

    pub(super) fn current_shop_wallet(&self) -> Wallet {
        let world = self.lot.world();
        let simoleons = sims_vm::available_funds(world).max(0);
        let fam_id = i32::from(world.globals.get(9).copied().unwrap_or(0));
        let members: Vec<u32> = world
            .families
            .iter()
            .find(|f| f.id == fam_id)
            .map(|f| f.members.clone())
            .unwrap_or_default();
        let magic_coins = sims_vm::travel::family_magic_coins(world, &members).max(0);
        Wallet {
            simoleons,
            magic_coins,
        }
    }

    pub(super) fn deduct_shop_currency(
        &mut self,
        currency: CurrencyKind,
        price: i32,
    ) -> Result<(), String> {
        if price == 0 {
            return Ok(());
        }
        self.mutate_world_via_saved(|world| {
            let fam_id = i32::from(world.globals.get(9).copied().unwrap_or(0));
            match currency {
                CurrencyKind::Simoleons => {
                    let next_funds = world.budget.funds.saturating_sub(price).max(0);
                    let _ = world.set_funds(next_funds);
                }
                CurrencyKind::MagiCoins => {
                    let members: Vec<u32> = world
                        .families
                        .iter()
                        .find(|f| f.id == fam_id)
                        .map(|f| f.members.clone())
                        .unwrap_or_default();
                    let mut remaining = price.max(0);
                    for guid in members {
                        if remaining == 0 {
                            break;
                        }
                        let Some(nid) = world
                            .neighbors
                            .slots
                            .iter()
                            .flatten()
                            .find(|r| r.guid == guid)
                            .map(|r| r.id)
                        else {
                            continue;
                        };
                        let have = i32::from(sims_vm::travel::neighbor_token_count(
                            world,
                            nid,
                            sims_vm::travel::MAGICOIN_TOKEN_KIND,
                            sims_vm::travel::MAGICOIN_TOKEN_GUID,
                        ));
                        if have > 0 {
                            let take = have.min(remaining);
                            let next = u16::try_from((have - take).max(0)).unwrap_or(0);
                            sims_vm::travel::set_neighbor_token_count(
                                world,
                                nid,
                                sims_vm::travel::MAGICOIN_TOKEN_KIND,
                                sims_vm::travel::MAGICOIN_TOKEN_GUID,
                                next,
                            );
                            remaining -= take;
                        }
                    }
                }
            }
        })
    }

    pub(super) fn ensure_shop_assets(&mut self) {
        if self.shop_strings.is_none() {
            let strings = ShopStrings::load(self.hud.install(), US_ENGLISH).unwrap_or_default();
            self.shop_strings = Some(strings);
        }
        if self.shop_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::shop::load_art(&ui, US_ENGLISH)
        {
            self.shop_art = Some(art);
        }
    }

    pub(super) fn draw_shop_dialog(&mut self) {
        if !self.shop_dialog.is_open() {
            return;
        }
        self.ensure_shop_assets();
        let Some(snapshot) = self.shop_snapshot.as_ref() else {
            return;
        };
        let strings = self.shop_strings.clone().unwrap_or_default();
        let art = self.shop_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        self.shop_dialog.draw(&mut frame, snapshot, &strings, art);
        self.canvas.color = frame.into_packed();
    }

    pub(super) fn draw_destination_picker(&mut self) {
        if self.destination_picker.is_none() {
            return;
        }
        if self.destination_picker_assets.is_none() {
            self.destination_picker_assets =
                Some(DestinationPickerAssets::load(self.hud.install()));
        }
        let Some(assets) = self.destination_picker_assets.as_ref() else {
            return;
        };
        if let Some(picker) = self.destination_picker.as_mut() {
            picker.draw(&mut self.canvas, assets, self.clock_us);
        }
    }
}
