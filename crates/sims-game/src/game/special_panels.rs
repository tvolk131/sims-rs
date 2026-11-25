//! Special dialog panels (`Dialog` kinds `5..=15`), their pickers, the phonebook and photos.

use super::*;

impl Game {
    // -------------------------------------------------------------------------
    // Special Dialog Panels (`Dialog` kinds `5..=15`, single-player & co-op)
    // -------------------------------------------------------------------------

    /// Opens `request`'s panel over `world` without the VM asking, so
    /// headless snapshot tests can draw it. Nothing answers the VM: picks
    /// are refused as unasked, and Escape closes the panel.
    #[doc(hidden)]
    pub fn preview_special_panel(&mut self, request: SpecialPanelRequest, world: &sims_sim::World) {
        self.close_special_panel_ui();
        self.preview_reply = None;
        self.open_special_panel_ui(0, request, world, None, true);
    }

    /// What the last preview panel would have answered the VM, had a request
    /// opened it: the reply [`Game::answer_special_panel`] got while
    /// previewing.
    #[doc(hidden)]
    pub fn preview_special_panel_reply(&self) -> Option<&SpecialPanelResponse> {
        self.preview_reply.as_ref()
    }

    /// Whether the open special panel is a synthetic preview rather than a
    /// VM request.
    pub(super) fn previewing_special_panel(&self) -> bool {
        self.active_special_panel
            .as_ref()
            .is_some_and(|p| p.preview)
    }

    /// Closes any local UI modal opened for a VM special panel without sending
    /// an answer to the VM (used after answering, on co-op peer dismissal, or on
    /// restore).
    pub(super) fn close_special_panel_ui(&mut self) {
        if let Some(panel) = self.active_special_panel.take() {
            if panel.preview {
                // The pointer last moved over the preview, not the lot:
                // listing resumes only on fresh pointer input, so dismissing
                // never runs a check tree for whatever was under it.
                self.pointer = None;
                self.reached = None;
                self.listed = None;
            }
            self.shop_dialog.close();
            self.shop_snapshot = None;
            self.shop_preview_yaw_deg = 0;
            self.shop_preview_body = None;
            self.shop_preview_targets.clear();
            self.book_dialog.close();
            self.book_snapshot = None;
            self.phonebook_dialog.close();
            self.phonebook_snapshot = None;
            self.destination_picker = None;
            self.hud.close_special_book_dialog();
        }
    }

    /// Synchronizes local modal UI with `Lot::asking_special_panel()` so all
    /// eleven special `Dialog` kinds (`5..=15`) — destination lot pickers,
    /// interactive wardrobe/pet/neighbor/charm pickers, and inline reference
    /// books (Spellbook 12 & Cookbook 15) — present clean native modals in both
    /// single-player and co-op sessions.
    pub fn sync_special_panel(&mut self) {
        // Borrow first to avoid cloning the paused VM world on unchanged frames.
        let coop_dialog_id = self
            .lot
            .coop()
            .and_then(|c| c.active_special_panel.as_ref().map(|(id, _)| *id));
        let unchanged = {
            let Some(asking) = self.lot.asking_special_panel() else {
                if self.previewing_special_panel() {
                    return;
                }
                self.submitted_coop_special_panel = None;
                self.close_special_panel_ui();
                return;
            };
            if coop_dialog_id.is_some()
                && self.submitted_coop_special_panel.map(|(id, _)| id) == coop_dialog_id
            {
                return;
            }
            self.active_special_panel
                .as_ref()
                .is_some_and(|a| a.shows(&asking.shown, coop_dialog_id))
        };
        if unchanged {
            return;
        }
        self.submitted_coop_special_panel = None;
        let Some(asking) = self.lot.asking_special_panel().cloned() else {
            return;
        };
        let tick = asking.shown.tick;
        let request = asking.shown.request.clone();
        self.open_special_panel_ui(tick, request, &asking.world, coop_dialog_id, false);
    }

    /// Opens the local modal for `request`, as the VM asked it at `tick`
    /// with `world` as the asking tick left it.
    pub(super) fn open_special_panel_ui(
        &mut self,
        tick: u32,
        request: SpecialPanelRequest,
        world: &sims_sim::World,
        coop_dialog_id: Option<NetDialogId>,
        preview: bool,
    ) {
        self.close_menu();
        self.hud.close_camera_strip();
        self.hud.close_special_book_dialog();
        self.book_dialog.close();
        self.book_snapshot = None;
        self.phonebook_dialog.close();
        self.phonebook_snapshot = None;
        self.destination_picker = None;
        self.shop_dialog.close();
        self.shop_snapshot = None;
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;

        match &request {
            SpecialPanelRequest::SpellBook { caller }
            | SpecialPanelRequest::CookBook { caller } => {
                let is_cookbook = matches!(request, SpecialPanelRequest::CookBook { .. });
                self.ensure_book_assets();
                let snapshot = self.build_reference_book_snapshot(world, *caller, is_cookbook);
                self.book_snapshot = Some(snapshot);
                self.book_dialog.open();
                self.active_special_panel = Some(ActiveSpecialPanel {
                    tick,
                    request,
                    coop_dialog_id,
                    responses: BTreeMap::new(),
                    preview_targets: BTreeMap::new(),
                    preview,
                });
            }
            SpecialPanelRequest::DowntownLotPicker
            | SpecialPanelRequest::VacationLotPicker { .. }
            | SpecialPanelRequest::OldTownLotPicker { .. }
            | SpecialPanelRequest::StudioTownLotPicker { .. }
            | SpecialPanelRequest::MagicTownLotPicker { .. } => {
                let entries = self.resolve_special_lot_entries();
                let wallet = self.current_shop_wallet();
                let fame_half_stars = world
                    .selected()
                    .and_then(|sel| world.objects.get(&sel))
                    .and_then(|o| o.person.as_ref())
                    .and_then(|p| p.data.get(81).copied())
                    .map(|level| level.clamp(0, 10) as u8)
                    .unwrap_or(0);
                if let Ok(picker) = DestinationMapPicker::open(
                    self.hud.install(),
                    self.neighborhood_slot,
                    request.clone(),
                    entries,
                    wallet.simoleons,
                    wallet.magic_coins,
                    fame_half_stars,
                ) {
                    if let Some(err) = picker.map_error() {
                        self.note = Some(format!(
                            "Could not load destination map for special panel kind {}: {err}",
                            request.kind()
                        ));
                    }
                    self.destination_picker = Some(picker);
                    self.active_special_panel = Some(ActiveSpecialPanel {
                        tick,
                        request,
                        coop_dialog_id,
                        responses: BTreeMap::new(),
                        preview_targets: BTreeMap::new(),
                        preview,
                    });
                }
            }
            SpecialPanelRequest::WardrobeShop { person, mode } => {
                // `0x5097c9` and the letter table at `0x6c0fac`: mode 1
                // lists formal (`F`) and mode 2 swimwear (`S`).
                let category = match *mode {
                    1 => WardrobeCategory::Formal,
                    2 => WardrobeCategory::Swimwear,
                    3 => WardrobeCategory::Pajamas,
                    4 => WardrobeCategory::Winterwear,
                    _ => WardrobeCategory::Everyday,
                };
                let shop_kind = if *mode == 5 {
                    ShopKind::StudioWardrobe
                } else {
                    ShopKind::DowntownBoutique(category)
                };
                let shopper = self.person_display_name(world, *person);
                let (items, responses, preview_targets) =
                    self.build_wardrobe_picker_items(world, *person, *mode);
                let current = self.current_look(world, *person, wardrobe_mode_row(*mode), None);
                let shapes = items
                    .iter()
                    .map(|item| {
                        preview_targets
                            .get(&item.id)
                            .map(|t| body_shape(&t.suit).to_owned())
                            .unwrap_or_default()
                    })
                    .collect();
                self.open_special_picker(
                    tick,
                    request,
                    coop_dialog_id,
                    shop_kind,
                    None,
                    shopper,
                    items,
                    responses,
                    preview_targets,
                    shapes,
                    ChoiceOpening::Current(current),
                    preview,
                );
            }
            SpecialPanelRequest::PetPicker { pet } => {
                let shopper = self.person_display_name(world, *pet);
                let (items, responses, preview_targets) = self.build_pet_picker_items(world, *pet);
                let skeleton = world
                    .objects
                    .get(pet)
                    .and_then(|o| self.person_body_string(o.guid, 0))
                    .filter(|s| !s.trim().is_empty());
                let current = self.current_look(world, *pet, 1, skeleton);
                // Pets' arrows skip no body shape (`0x40d265`: kind 2 and up).
                self.open_special_picker(
                    tick,
                    request,
                    coop_dialog_id,
                    ShopKind::PetAdoption,
                    None,
                    shopper,
                    items,
                    responses,
                    preview_targets,
                    Vec::new(),
                    ChoiceOpening::Current(current),
                    preview,
                );
            }
            SpecialPanelRequest::CallNeighbor { caller_neighbor } => {
                // Native `cWinPhoneBook` (`0x5221d0`) on `phonebookbkg.bmp`.
                self.ensure_phonebook_assets();
                let snapshot = self.build_phonebook_snapshot(world, *caller_neighbor);
                self.phonebook_snapshot = Some(snapshot);
                self.phonebook_dialog.open();
                self.active_special_panel = Some(ActiveSpecialPanel {
                    tick,
                    request,
                    coop_dialog_id,
                    responses: BTreeMap::new(),
                    preview_targets: BTreeMap::new(),
                    preview,
                });
            }
            SpecialPanelRequest::CharmTransform { person } => {
                let shopper = self.person_display_name(world, *person);
                let (items, responses) = Self::build_charm_transform_items(world, *person);
                // The costumes are dressed by the VM's suit table
                // (`0x58a948..0x58d475`), which the preview cannot reach:
                // the view names the costume instead.
                self.open_special_picker(
                    tick,
                    request,
                    coop_dialog_id,
                    ShopKind::MagicVendor,
                    None,
                    shopper,
                    items,
                    responses,
                    BTreeMap::new(),
                    Vec::new(),
                    ChoiceOpening::First,
                    preview,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn open_special_picker(
        &mut self,
        tick: u32,
        request: SpecialPanelRequest,
        coop_dialog_id: Option<NetDialogId>,
        kind: ShopKind,
        title_override: Option<String>,
        shopper_name: Option<String>,
        items: Vec<ShopItem>,
        responses: BTreeMap<String, SpecialPanelResponse>,
        preview_targets: BTreeMap<String, ShopPreviewTarget>,
        body_shapes: Vec<String>,
        opening: ChoiceOpening,
        preview: bool,
    ) {
        self.ensure_shop_assets();
        let wallet = self.current_shop_wallet();
        // The Wardrobe opens `cWinClothingChoiceDlg` (`0x5090d0`), the Charm
        // `cWinCharmDlg` (`0x536a6b`) and the pet pen the pet dialog
        // (`0x521911`): one 3D view between arrows.
        let choice = match request {
            SpecialPanelRequest::WardrobeShop { .. } => Some(ChoiceDialog::Clothing),
            SpecialPanelRequest::CharmTransform { .. } => Some(ChoiceDialog::Charm),
            SpecialPanelRequest::PetPicker { .. } => Some(ChoiceDialog::Pet),
            _ => None,
        };
        // A choice dialog's view opens on the person as they are
        // (`0x56d3d9` copies them), and OK answers the first item equal to
        // the view's outfit (`0x56d852` → `0x40d0a4` and its kin compare
        // suit and texture, ignoring case): the first item equal to the
        // person's current row, or none (`-1`).
        // The Charm's view opens on its first preset instead (`0x536afe`
        // zeroes it, `0x5372d9` shows it).
        let (current, shown_id) = match opening {
            ChoiceOpening::Current(current) => {
                let shown = choice.and(current.as_ref()).and_then(|look| {
                    let key = outfit_key(look.row.as_deref()?);
                    items
                        .iter()
                        .find(|item| {
                            preview_targets
                                .get(&item.id)
                                .and_then(|t| t.row.as_deref())
                                .is_some_and(|row| outfit_key(row) == key)
                        })
                        .map(|item| item.id.clone())
                });
                (current, shown)
            }
            ChoiceOpening::First => (None, items.first().map(|item| item.id.clone())),
        };
        self.shop_snapshot = Some(ShopSnapshot {
            kind,
            title_override,
            buy_label_override: None,
            shopper_name,
            wallet,
            items,
            tried_on_item_id: None,
            preview_image: None,
            preview_label: None,
            // Special VM pickers carry no price: the calling object's own
            // tree charges (if anything), so never show a bogus `$0` wallet.
            hide_wallet_and_prices: true,
            choice,
            body_shapes,
        });
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        self.shop_dialog.open();
        // A new request never inherits the previous request's selection.
        if choice.is_some() {
            match &shown_id {
                Some(id) => self.shop_dialog.select_item(id.clone()),
                None => self.shop_dialog.clear_selection(),
            }
        }
        let shown = shown_id.map(|id| (preview_targets.get(&id).cloned(), id));
        self.active_special_panel = Some(ActiveSpecialPanel {
            tick,
            request,
            coop_dialog_id,
            responses,
            preview_targets,
            preview,
        });
        match (choice, shown, current) {
            (Some(_), Some((target, id)), _) => self.apply_shop_try_on(&id, target.as_ref()),
            (Some(_), None, current) => self.show_current_look(current.as_ref()),
            (None, ..) => {}
        }
    }

    /// The person's look a choice dialog opens on: their body string `row`
    /// (`1` everyday, `30..=34` formal, swim, pajamas, winter and high
    /// fashion; native passes the 1-based slot to `0x472ec4`), as live as
    /// the port keeps it (strings Make New Character wrote, then rows the
    /// clothing initialization filled, then the type's own `STR# 200`).
    pub(super) fn current_look(
        &self,
        world: &sims_sim::World,
        person: u16,
        row: usize,
        pet_skeleton: Option<String>,
    ) -> Option<ShopPreviewTarget> {
        let guid = world.objects.get(&person)?.guid;
        let raw = world
            .characters
            .made
            .get(&guid)
            .and_then(|c| c.strings.get(&row))
            .or_else(|| world.made_outfits.get(&guid).and_then(|m| m.get(&row)))
            .cloned()
            .or_else(|| self.person_body_string(guid, row))?;
        let (suit, _) = outfit_key(&raw);
        Some(ShopPreviewTarget {
            person,
            suit,
            row: Some(raw),
            pet_skeleton,
        })
    }

    /// Shows the person as they are in a choice dialog's view, with no
    /// item chosen.
    pub(super) fn show_current_look(&mut self, look: Option<&ShopPreviewTarget>) {
        self.shop_preview_yaw_deg = 0;
        self.shop_preview_body = None;
        let image = look.and_then(|look| self.render_shop_try_on_image("", Some(look)));
        if let Some(snap) = &mut self.shop_snapshot {
            snap.tried_on_item_id = None;
            snap.preview_label = Some(if image.is_some() {
                "Current outfit".to_owned()
            } else {
                "Current outfit (preview unavailable)".to_owned()
            });
            snap.preview_image = image;
        }
        self.note = Some("Previewing the current outfit.".to_owned());
    }

    /// The 3D preview's parameters at `yaw`: a choice dialog's view is
    /// 200 by 300 (`0x509779`), and the pet dialog builds its view with
    /// 5.0 (`0x667678`) where the clothing dialog and Create-a-Sim pass
    /// 3.25 (`0x66ad3c`, `0x53724f`), so pets draw that much larger.
    pub(super) fn shop_preview_params(
        &self,
        yaw: f32,
    ) -> sims_people::caf_preview::TurntableParams {
        let defaults = sims_people::caf_preview::TurntableParams {
            yaw,
            ..sims_people::caf_preview::TurntableParams::default()
        };
        match self.shop_snapshot.as_ref().and_then(|s| s.choice) {
            Some(ChoiceDialog::Clothing | ChoiceDialog::Charm) => {
                sims_people::caf_preview::TurntableParams {
                    size: sims_ui::shop::CHOICE_VIEW,
                    ..defaults
                }
            }
            Some(ChoiceDialog::Pet) => sims_people::caf_preview::TurntableParams {
                size: sims_ui::shop::CHOICE_VIEW,
                scale: defaults.scale * 5.0 / 3.25,
                ..defaults
            },
            None => defaults,
        }
    }

    /// Delivers `response` to the currently waiting VM special panel (`Dialog`
    /// kinds `5..=15`) in either single-player (`Lot::answer_special_panel` +
    /// tick replay) or co-op lockstep (`sims_vm::coop::special_panel_answer_command`).
    pub fn answer_special_panel(&mut self, response: SpecialPanelResponse) -> bool {
        if self.previewing_special_panel() {
            // A synthetic preview panel never answers a real VM request,
            // even one that happens to be waiting.
            self.preview_reply = Some(response);
            self.close_special_panel_ui();
            return false;
        }
        let Some(asking) = self.lot.asking_special_panel() else {
            self.close_special_panel_ui();
            self.note = Some("No special panel waits for a response.".into());
            return false;
        };
        let request = asking.shown.request.clone();
        if !request.accepts(&response) {
            self.note = Some(format!(
                "Special panel kind {} does not accept {response:?}.",
                request.kind()
            ));
            return false;
        }
        if self.coop.is_active() {
            let dialog_id = self
                .lot
                .coop()
                .and_then(|c| c.active_special_panel.as_ref().map(|(id, _)| *id))
                .or_else(|| {
                    self.active_special_panel
                        .as_ref()
                        .and_then(|a| a.coop_dialog_id)
                })
                .or_else(|| {
                    self.coop
                        .session()
                        .and_then(|s| s.open_dialogs.keys().next().copied())
                });
            let Some(dialog_id) = dialog_id else {
                self.note = Some("No co-op blocking special panel is active.".into());
                return false;
            };
            let cmd = sims_vm::coop::special_panel_answer_command(dialog_id, &response);
            if let Err(e) = self.coop.submit_command(cmd) {
                self.note = Some(format!("Special panel response was not sent: {e}"));
                return false;
            }
            self.submitted_coop_special_panel = Some((dialog_id, 0));
            self.close_special_panel_ui();
            return true;
        }
        if let Err(e) = self.lot.answer_special_panel(response) {
            self.note = Some(format!("Special panel response was not taken: {e}"));
            return false;
        }
        self.close_special_panel_ui();
        // Replay the paused tick immediately with the chosen response, just
        // like `Game::answer` does for ordinary dialogs. Committed destination
        // lot picks are recorded into `pending_travel_lot` inside `run_while`
        // from `Update::special_panels_answered` only when the tick commits.
        self.run(1);
        self.sync_special_panel();
        true
    }

    pub(super) fn execute_special_panel_shop_command(
        &mut self,
        active: ActiveSpecialPanel,
        cmd: ShopCommand,
    ) -> Outcome {
        match cmd {
            ShopCommand::Cancel => {
                let mut cancel = special_panel_cancel_response(&active.request);
                // Cancel also sends the view's index (`0x5098d1`, `0x522087`):
                // the shown item's, or `-1` with none.
                if let SpecialPanelResponse::Selection { index, .. } = &mut cancel
                    && let Some(snapshot) = &self.shop_snapshot
                    && snapshot.choice.is_some()
                {
                    *index = self
                        .shop_dialog
                        .chosen_item(snapshot)
                        .and_then(|item| active.responses.get(&item.id))
                        .map_or(-1, |resp| match resp {
                            SpecialPanelResponse::Selection { index, .. } => *index,
                            _ => -1,
                        });
                }
                if self.answer_special_panel(cancel) {
                    self.hud.play(crate::hud::CLICK);
                } else {
                    self.hud.play(crate::hud::ERROR);
                    // Re-open the picker if co-op submission failed so the
                    // blocking special panel never stays hidden.
                    if self.shop_snapshot.is_some() {
                        self.shop_dialog.open();
                    } else {
                        self.active_special_panel = None;
                        self.sync_special_panel();
                    }
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::TryOn { item_id } => {
                let target = active.preview_targets.get(&item_id);
                self.apply_shop_try_on(&item_id, target);
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
            ShopCommand::NoSelection => {
                // OK with none of the items in the view: index `-1`.
                let mut keep = special_panel_cancel_response(&active.request);
                if let SpecialPanelResponse::Selection { accepted, index } = &mut keep {
                    *accepted = true;
                    *index = -1;
                    if self.answer_special_panel(keep) {
                        self.hud.play(crate::hud::CLICK);
                    } else {
                        self.hud.play(crate::hud::ERROR);
                        if self.shop_snapshot.is_some() {
                            self.shop_dialog.open();
                        } else {
                            self.active_special_panel = None;
                            self.sync_special_panel();
                        }
                    }
                } else {
                    self.hud.play(crate::hud::ERROR);
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            ShopCommand::Buy { item_id, .. } => {
                if let Some(resp) = active.responses.get(&item_id).cloned() {
                    if self.answer_special_panel(resp) {
                        self.hud.play(crate::hud::CLICK);
                    } else {
                        self.hud.play(crate::hud::ERROR);
                        if self.shop_snapshot.is_some() {
                            self.shop_dialog.open();
                        } else {
                            self.active_special_panel = None;
                            self.sync_special_panel();
                        }
                    }
                } else {
                    self.hud.play(crate::hud::ERROR);
                }
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
        }
    }

    pub(super) fn person_display_name(
        &self,
        world: &sims_sim::World,
        object_id: u16,
    ) -> Option<String> {
        let obj = world.objects.get(&object_id)?;
        self.lot
            .character_names()
            .get(&obj.guid)
            .cloned()
            .or_else(|| self.hud.name(obj.guid).map(str::to_owned))
    }

    pub(super) fn build_reference_book_snapshot(
        &self,
        world: &sims_sim::World,
        caller: Option<u16>,
        is_cookbook: bool,
    ) -> ReferenceBookSnapshot {
        let strings = self.book_strings.clone().unwrap_or_default();
        // `magic.iff STR# 1[0..1]` is `"%s Family Spellbook"` / `"%s Family
        // Cookbook"`: the subject is the caller's family surname (`FAMs` of the
        // family `person.data[61]` names), e.g. `"Goth Family Spellbook"`.
        let surname = caller
            .and_then(|id| world.objects.get(&id))
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(sims_sim::neighbors::FAMILY).copied())
            .filter(|&fam| fam > 0)
            .and_then(|fam| self.hud.family_surname(i32::from(fam)))
            .map(str::to_owned);
        let title = strings.format_title(is_cookbook, surname.as_deref());

        let neighbor_id = caller
            .and_then(|id| world.objects.get(&id))
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(31).copied())
            .filter(|&nid| nid > 0);
        let mut held_tokens: Vec<(String, u16)> = Vec::new();
        if let Some(nid) = neighbor_id
            && let Some(inv) = world
                .neighbors
                .inventories
                .iter()
                .find(|i| i.neighbor == nid)
        {
            for item in &inv.items {
                if item.count == 0 {
                    continue;
                }
                // Kind 7 holds magic tokens/spells/ingredients; kind 8/9 hold recipes/charms.
                let relevant = if is_cookbook {
                    matches!(item.kind, 7..=9)
                } else {
                    matches!(item.kind, 7 | 8)
                };
                if !relevant {
                    continue;
                }
                let label = self
                    .catalog_object_name(item.guid)
                    .unwrap_or_else(|| format!("0x{:08X}", item.guid));
                held_tokens.push((label, item.count));
            }
        }

        let held_summary: Vec<BookBadge> = held_tokens
            .iter()
            .map(|(name, count)| BookBadge {
                label: if *count > 1 {
                    format!("{name} (x{count})")
                } else {
                    name.clone()
                },
                owned: true,
            })
            .collect();

        let catalog =
            sims_ui::book::load_magic_book_entries(self.hud.install(), is_cookbook, US_ENGLISH)
                .unwrap_or_default();

        let mut pages: Vec<BookPage> = Vec::new();
        for entry in catalog {
            let haystack = format!(
                "{} {}",
                entry.title.to_ascii_lowercase(),
                entry.description.to_ascii_lowercase()
            );
            let mut matched_badges = Vec::new();
            for (token_name, count) in &held_tokens {
                let needle = token_name.to_ascii_lowercase();
                if needle.len() >= 3 && haystack.contains(&needle) {
                    matched_badges.push(BookBadge {
                        label: if *count > 1 {
                            format!("{token_name} (x{count})")
                        } else {
                            token_name.clone()
                        },
                        owned: true,
                    });
                }
            }
            let highlight = !matched_badges.is_empty();
            pages.push(BookPage {
                heading: None,
                category: Some(entry.category),
                entries: vec![BookEntry {
                    title: entry.title,
                    subtitle: None,
                    body: entry.description,
                    badges: matched_badges,
                    highlight,
                }],
            });
        }

        if pages.is_empty() {
            let body = if held_summary.is_empty() {
                "—".to_owned()
            } else {
                held_summary
                    .iter()
                    .map(|b| b.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            pages.push(BookPage {
                heading: None,
                category: None,
                entries: vec![BookEntry {
                    title: title.clone(),
                    subtitle: None,
                    body,
                    badges: Vec::new(),
                    highlight: !held_summary.is_empty(),
                }],
            });
        }

        ReferenceBookSnapshot {
            title,
            subtitle: None,
            held_summary,
            pages,
            use_magic_book_chrome: true,
        }
    }

    pub(super) fn catalog_object_name(&self, guid: u32) -> Option<String> {
        let catalog = self.assets.catalog();
        let obj = catalog
            .by_guid
            .get(&guid)
            .and_then(|&idx| catalog.objects.get(idx))?;
        let bytes = self
            .assets
            .index()
            .resources
            .get(obj.resource)?
            .read()
            .ok()?;
        let iff = sims_formats::iff::Iff::parse(bytes).ok()?;
        if let Some(chunk) = iff.get(b"OBJD", obj.chunk) {
            if let Ok(def) = sims_formats::behavior::ObjectDefinition::parse(iff.data(chunk)) {
                let ctss_id = def.catalog;
                if ctss_id > 0
                    && let Some(ctss) = iff.get(b"CTSS", ctss_id)
                    && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(ctss))
                    && let Some(name) = table
                        .get(0, US_ENGLISH)
                        .or_else(|| table.get(0, 1))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                {
                    return Some(name.to_owned());
                }
            }
            let chunk_label = chunk.label.trim();
            if !chunk_label.is_empty() {
                return Some(chunk_label.to_owned());
            }
        }
        None
    }

    pub(super) fn resolve_special_lot_entries(&self) -> Vec<SpecialLotEntry> {
        if !self.special_lot_catalog.is_empty() {
            return self.special_lot_catalog.clone();
        }
        let install = self.hud.install();
        let user_dir = install
            .file("UserData")
            .or_else(|_| install.file("TemplateUserData"))
            .unwrap_or_else(|_| install.root.clone());
        let street_names = crate::neighborhood::StreetNames::load(install, &user_dir);
        let descriptions = crate::neighborhood::NeighborhoodDescriptions::load(install, &user_dir);
        let mut zoning: BTreeMap<u16, i16> = BTreeMap::new();
        let mut discovered: BTreeSet<u16> = BTreeSet::new();
        let expansions = u16::from(install.expansions.0);
        if let Ok(nb) = Neighborhood::open(&user_dir) {
            if let Ok(houses) = nb.houses() {
                for (num, _) in houses {
                    if let Ok(id) = u16::try_from(num) {
                        discovered.insert(id);
                    }
                }
            }
            if let Ok(nb_zoning) = nb.zoning(expansions) {
                for (lot, zone) in nb_zoning {
                    if let Ok(id) = u16::try_from(lot) {
                        zoning.insert(id, zone as i16);
                    }
                }
            }
        }
        if let Ok(tpl_dir) = install.file("TemplateUserData")
            && let Ok(nb) = Neighborhood::open(&tpl_dir)
            && let Ok(houses) = nb.houses()
        {
            for (num, _) in houses {
                if let Ok(id) = u16::try_from(num) {
                    discovered.insert(id);
                }
            }
            if let Ok(nb_zoning) = nb.zoning(expansions) {
                for (lot, zone) in nb_zoning {
                    if let Ok(id) = u16::try_from(lot) {
                        zoning.entry(id).or_insert(zone as i16);
                    }
                }
            }
        }
        discovered
            .into_iter()
            .map(|lot_id| {
                let (name, description) = descriptions.lot_info(lot_id);
                let address = street_names.address_for(lot_id);
                let z = zoning.get(&lot_id).copied().unwrap_or(match lot_id {
                    21..=30 | 40..=48 | 81..=89 | 93..=98 => 2,
                    _ => 1,
                });
                SpecialLotEntry {
                    lot_id,
                    name,
                    address,
                    description,
                    zoning: z,
                }
            })
            .collect()
    }

    pub(super) fn person_skin_tone(&self, world: &sims_sim::World, person: u16) -> String {
        let Some(obj) = world.objects.get(&person) else {
            return "lgt".into();
        };
        let normalize = |raw: &str| -> Option<String> {
            let prefix: String = raw.trim().chars().take(3).collect();
            if prefix.eq_ignore_ascii_case("lgt")
                || prefix.eq_ignore_ascii_case("med")
                || prefix.eq_ignore_ascii_case("drk")
            {
                Some(prefix.to_ascii_lowercase())
            } else {
                None
            }
        };
        if let Some(outfits) = world.made_outfits.get(&obj.guid)
            && let Some(raw) = outfits.get(&14)
            && let Some(tone) = normalize(raw)
        {
            return tone;
        }
        let catalog = self.assets.catalog();
        if let Some(entry) = catalog
            .by_guid
            .get(&obj.guid)
            .and_then(|&idx| catalog.objects.get(idx))
            && let Some(res) = self.assets.index().resources.get(entry.resource)
            && let Ok(bytes) = res.read()
            && let Ok(iff) = sims_formats::iff::Iff::parse(bytes)
            && let Some(chunk) = iff.get(b"STR#", 200)
            && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk))
            && let Some(raw) = table.get(14, US_ENGLISH).or_else(|| table.get(14, 1))
            && let Some(tone) = normalize(raw)
        {
            return tone;
        }
        "lgt".into()
    }

    pub(super) fn ensure_ordered_wardrobe(&mut self) -> Option<&sims_content::ImportWardrobe> {
        if self.ordered_wardrobe.is_none()
            && let Ok((wardrobe, _)) =
                sims_content::ImportWardrobe::load_lenient(self.hud.install())
        {
            self.ordered_wardrobe = Some(wardrobe);
        }
        self.ordered_wardrobe.as_ref()
    }

    /// The caller's `(female, child)` from `person.data[65]` bit 0 and age
    /// word 58.
    pub(super) fn person_sex_and_age(world: &sims_sim::World, person: u16) -> (bool, bool) {
        world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                let kind = p.data.get(sims_sim::neighbors::KIND).copied().unwrap_or(0);
                let age = p.data.get(58).copied().unwrap_or(27);
                ((kind & 1) == 1, age < 18)
            })
            .unwrap_or((false, false))
    }

    /// `Dialog` kind 6 (`0x508f22`): every outfit of the mode's category the
    /// caller can wear — matching sex (or unisex), age and skin tone — in
    /// native wardrobe order, uncapped. Item `i` answers `Selection { index: i }`.
    pub(super) fn build_wardrobe_picker_items(
        &mut self,
        world: &sims_sim::World,
        person: u16,
        mode: i16,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let (female, child) = Self::person_sex_and_age(world, person);
        let skin = self.person_skin_tone(world, person);
        let outfits = self
            .ensure_ordered_wardrobe()
            .map(|w| category_outfits(w, wardrobe_mode_category(mode), female, child, &skin))
            .unwrap_or_default();
        Self::outfit_picker_items("special_wardrobe", person, outfits)
    }

    /// Picker rows for human outfits: clean labels, no price, try-on enabled.
    pub(super) fn outfit_picker_items(
        id_prefix: &str,
        person: u16,
        outfits: Vec<(String, String)>,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let mut items = Vec::with_capacity(outfits.len());
        let mut responses = BTreeMap::new();
        let mut preview_targets = BTreeMap::new();
        let mut first_of = BTreeMap::new();
        for (idx, (suit, row)) in outfits.into_iter().enumerate() {
            let Ok(index) = i16::try_from(idx) else {
                break;
            };
            // OK answers the first item equal to the view's outfit
            // (`0x40d0a4`), so a repeated outfit answers its first index.
            let index = *first_of.entry(outfit_key(&row)).or_insert(index);
            let id = format!("{id_prefix}:{idx}");
            items.push(ShopItem {
                id: id.clone(),
                name: outfit_display_label(&suit, &row),
                description: String::new(),
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: true,
                owned: false,
                available: true,
            });
            responses.insert(
                id.clone(),
                SpecialPanelResponse::Selection {
                    accepted: true,
                    index,
                },
            );
            preview_targets.insert(id, ShopPreviewTarget::human(person, suit, Some(row)));
        }
        (items, responses, preview_targets)
    }

    /// `Dialog` kind 9 (`0x521788`): the pet bodies of the pen pet's species
    /// in the exact `ImportWardrobe::pick_pet` order `Make New Character`
    /// reads the answer with (`0x44b064`), uncapped, with breed and sex labels
    /// and a 3D turntable preview on the pet's own skeleton.
    pub(super) fn build_pet_picker_items(
        &mut self,
        world: &sims_sim::World,
        pet: u16,
    ) -> (
        Vec<ShopItem>,
        BTreeMap<String, SpecialPanelResponse>,
        BTreeMap<String, ShopPreviewTarget>,
    ) {
        let kind = world
            .objects
            .get(&pet)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(sims_sim::neighbors::KIND).copied())
            .unwrap_or(0);
        let species = sims_ui::household::PetKind::from_kind(kind);
        let is_cat = species == Some(sims_ui::household::PetKind::Cat);
        let female = kind & 1 == 1;
        let skeleton = world
            .objects
            .get(&pet)
            .and_then(|o| self.person_body_string(o.guid, 0))
            .filter(|s| !s.trim().is_empty());
        let sex_species = match (female, is_cat) {
            (false, false) => "Male Dog",
            (true, false) => "Female Dog",
            (false, true) => "Male Cat",
            (true, true) => "Female Cat",
        };
        let rows: Vec<(String, String)> = self
            .ensure_ordered_wardrobe()
            .map(|w| {
                (0..w.count_pet(is_cat))
                    .map_while(|idx| w.pick_pet(is_cat, idx))
                    .collect()
            })
            .unwrap_or_default();
        let mut items = Vec::with_capacity(rows.len());
        let mut responses = BTreeMap::new();
        let mut preview_targets = BTreeMap::new();
        let mut first_of = BTreeMap::new();
        for (idx, (suit, row)) in rows.into_iter().enumerate() {
            let Ok(index) = i16::try_from(idx) else {
                break;
            };
            // `0x522069` resolves like the wardrobe: the first equal item.
            let index = *first_of.entry(outfit_key(&row)).or_insert(index);
            let id = format!("special_pet:{idx}");
            items.push(ShopItem {
                id: id.clone(),
                name: pet_breed_label(&row),
                description: sex_species.to_owned(),
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: skeleton.is_some(),
                owned: false,
                available: true,
            });
            responses.insert(
                id.clone(),
                SpecialPanelResponse::Selection {
                    accepted: true,
                    index,
                },
            );
            if let Some(skeleton) = &skeleton {
                preview_targets.insert(
                    id,
                    ShopPreviewTarget {
                        person: pet,
                        suit,
                        row: Some(row),
                        pet_skeleton: Some(skeleton.clone()),
                    },
                );
            }
        }
        (items, responses, preview_targets)
    }

    /// Body string `index` (`STR# 200`) of the person type `guid`, preferring
    /// strings Make New Character wrote.
    pub(super) fn person_body_string(&self, guid: u32, index: usize) -> Option<String> {
        let catalog = self.assets.catalog();
        let entry = catalog
            .by_guid
            .get(&guid)
            .and_then(|&idx| catalog.objects.get(idx))?;
        let bytes = self
            .assets
            .index()
            .resources
            .get(entry.resource)?
            .read()
            .ok()?;
        let iff = sims_formats::iff::Iff::parse(bytes).ok()?;
        let chunk = iff.get(b"STR#", 200)?;
        let table = sims_formats::strings::StringTable::parse(iff.data(chunk)).ok()?;
        table
            .get(index, US_ENGLISH)
            .or_else(|| table.get(index, 1))
            .map(|s| s.trim().to_owned())
    }

    /// `Dialog` kind 14 (`cWinCharmDlg`, `0x536a6b`): the Charm's eight
    /// costume presets, not the wardrobe. The preset starts at 0
    /// (`0x536afe`), the arrows step it through `0..=7` with wrap
    /// (`0x537384`, `0x5373a7`), and each preset is a suit number by the
    /// person's age and sex (`0x5374d3`, [`charm_suit`]). That number is
    /// what the view wears (`0x56d819` writes it to person word 8) and what
    /// OK and Cancel send (`0x5373df`, `0x5373fc`: `[view + 0x1b0]`).
    pub(super) fn build_charm_transform_items(
        world: &sims_sim::World,
        person: u16,
    ) -> (Vec<ShopItem>, BTreeMap<String, SpecialPanelResponse>) {
        let data = world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| p.data.as_slice())
            .unwrap_or_default();
        let word = |idx: usize| data.get(idx).copied().unwrap_or(0);
        // `0x5374de`: word 65 (`+0x60e`) equal to 1; `0x5374eb..0x5374fb`:
        // word 58 (`+0x600`, the age) nonzero and below 18, compared
        // signed (`jge` at `0x5374fb`), so a negative age is a child too.
        let female = word(sims_sim::neighbors::KIND) == 1;
        let age = word(58);
        let child = age != 0 && age < 18;
        let mut items = Vec::with_capacity(8);
        let mut responses = BTreeMap::new();
        for preset in 0..8 {
            let suit = charm_suit(preset, female, child);
            let id = format!("special_charm:{preset}");
            items.push(ShopItem {
                id: id.clone(),
                name: charm_costume_label(suit).to_owned(),
                description: String::new(),
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: true,
                owned: false,
                available: true,
            });
            responses.insert(
                id,
                SpecialPanelResponse::Selection {
                    accepted: true,
                    index: suit,
                },
            );
        }
        (items, responses)
    }

    /// Builds the call-eligible neighbor list for the native Phonebook
    /// (`cWinPhoneBook`, `Dialog` kind 10). A neighbor is listed when the
    /// caller has a relationship row naming them (`"They can only call people
    /// they've already met"`, `UIText.iff STR# 166[59]`), and they are not the
    /// caller, not in the caller's family (`person.data[61]`), not a pet
    /// (`person.data[65] > 1`, `0x427c69`) and not familyless (service NPC).
    /// Rows keep the caller's relationship order.
    ///
    /// Native lists neighbors on this lot too: `cWinPhoneBook` never asks
    /// where a neighbor is. It filters by family (`0x522ea1`), kind and a
    /// relationship row with values (`0x522f17`), and skips a neighbor whose
    /// GUID no longer resolves to a definition (`0x523011` after
    /// `0x43ac10`) or whose type's catalog name is empty (`0x523064`,
    /// read as native does by [`Game::catalog_name`]). See
    /// [`phonebook_eligible`]. Each target is judged by
    /// its first row, as `0x464f74` finds it, and listed once.
    ///
    /// **Not native:** `cWinPhoneBook` pages by family (`0x5230cc` picks
    /// the page's family, `0x522f7a` lists its members); this lists every
    /// eligible neighbor in the caller's relationship order.
    pub(super) fn build_phonebook_snapshot(
        &self,
        world: &sims_sim::World,
        caller_neighbor: i16,
    ) -> PhoneBookSnapshot {
        let strings = self.phonebook_strings.clone().unwrap_or_default();
        let records: BTreeMap<i16, &sims_sim::neighbors::Neighbor> = world
            .neighbors
            .slots
            .iter()
            .flatten()
            .map(|r| (r.id, r))
            .collect();
        let caller = records.get(&caller_neighbor).copied();
        let caller_name = caller.and_then(|r| self.neighbor_display_name(r.guid));
        let caller_family = caller
            .and_then(|r| r.data.get(sims_sim::neighbors::FAMILY).copied())
            .unwrap_or(0);
        let mut seen = BTreeSet::new();
        let entries = caller
            .map(|c| c.relationships.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|row| seen.insert(row.target))
            .filter_map(|row| {
                let id = i16::try_from(row.target).ok()?;
                let record = records.get(&id).copied()?;
                let resolves = self
                    .lot
                    .catalog()
                    .object(world.characters.definition(record.guid))
                    .is_some();
                let eligible = phonebook_eligible(
                    record,
                    caller_neighbor,
                    caller_family,
                    &world.families,
                    row,
                ) && resolves;
                if !eligible {
                    return None;
                }
                // `0x52304c..0x523064`: a neighbor whose type has an empty
                // catalog name (`0x4573d9`) is not listed. The name falls
                // back to `*` and the definition's label, so only a
                // definition the port cannot read is left out.
                let name = self.catalog_name(world, record.guid);
                if name.is_empty() {
                    return None;
                }
                Some({
                    let daily = row.values.first().copied().unwrap_or(0);
                    let lifetime = row.values.get(1).copied().unwrap_or(0);
                    let clamp = |v: i32| v.clamp(-100, 100) as i16;
                    PhoneBookEntry {
                        neighbor_id: id,
                        name,
                        daily: clamp(daily),
                        lifetime: clamp(lifetime),
                        portrait: self.hud.face(record.guid).cloned(),
                        is_friend: daily >= 50 || lifetime >= 50,
                    }
                })
            })
            .collect();
        let empty_message = match caller_name.as_deref() {
            Some(name) => format!("{name} {}", strings.knows_nobody),
            None => strings.knows_nobody.clone(),
        };
        PhoneBookSnapshot {
            title: strings.title.clone(),
            caller_name,
            entries,
            cancel_label: strings.cancel.clone(),
            prev_label: "<".into(),
            next_label: ">".into(),
            empty_message,
        }
    }

    /// The catalog name of person type `guid`, as `0x4573d9` reads it for
    /// the Phonebook's check (`0x52304c`): [`definition_catalog_name`] over
    /// the definition's `CATS` or `CTSS` and its `OBJD` label. A type Make
    /// New Character made reads the name it was given (`0x44b0e8`), which
    /// the port keeps instead of a file. Empty only when the definition
    /// cannot be read; native has no such case.
    pub(super) fn catalog_name(&self, world: &sims_sim::World, guid: u32) -> String {
        if let Some(name) = self.lot.character_names().get(&guid)
            && !name.is_empty()
        {
            return name.clone();
        }
        let catalog = self.assets.catalog();
        let Some(object) = catalog.object(world.characters.definition(guid)) else {
            return String::new();
        };
        let Some(bytes) = self
            .assets
            .index()
            .resources
            .get(object.resource)
            .and_then(|r| r.read().ok())
        else {
            return String::new();
        };
        let Ok(iff) = sims_formats::iff::Iff::parse(bytes) else {
            return String::new();
        };
        let Some(chunk) = iff.get(b"OBJD", object.chunk) else {
            return String::new();
        };
        // Definition word 41 (`+0x52`) names the catalog strings.
        let table = iff
            .data(chunk)
            .get(0x52..0x54)
            .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let (cats, ctss) = catalog_strings(&iff, table);
        definition_catalog_name(cats, ctss.as_ref(), object.kind == 2, &chunk.label)
    }

    pub(super) fn neighbor_display_name(&self, guid: u32) -> Option<String> {
        self.lot
            .character_names()
            .get(&guid)
            .cloned()
            .or_else(|| self.hud.name(guid).map(str::to_owned))
    }

    pub(super) fn ensure_phonebook_assets(&mut self) {
        if self.phonebook_strings.is_none() {
            let strings =
                PhoneBookStrings::load(self.hud.install(), US_ENGLISH).unwrap_or_default();
            self.phonebook_strings = Some(strings);
        }
        if self.phonebook_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::phonebook::load_art(&ui, US_ENGLISH)
        {
            self.phonebook_art = Some(art);
        }
    }

    pub(super) fn draw_phonebook_dialog(&mut self) {
        if !self.phonebook_dialog.is_open() {
            return;
        }
        self.ensure_phonebook_assets();
        let Some(snapshot) = self.phonebook_snapshot.as_ref() else {
            return;
        };
        let art = self.phonebook_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        self.phonebook_dialog.draw(&mut frame, snapshot, art);
        self.canvas.color = frame.into_packed();
    }

    /// The open Phonebook's view-model, if any (for tests and the harness).
    pub fn phonebook_snapshot(&self) -> Option<&PhoneBookSnapshot> {
        self.phonebook_dialog
            .is_open()
            .then_some(self.phonebook_snapshot.as_ref())
            .flatten()
    }

    /// Calls the Phonebook's selected neighbor (`Enter`).
    pub fn phonebook_submit(&mut self) -> Outcome {
        let cmd = self
            .phonebook_snapshot
            .as_ref()
            .and_then(|snap| self.phonebook_dialog.submit(snap));
        match cmd {
            Some(cmd) => self.execute_phonebook_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Cancels the Phonebook (`Escape`).
    pub fn phonebook_escape(&mut self) -> Outcome {
        match self.phonebook_dialog.escape() {
            Some(cmd) => self.execute_phonebook_command(cmd),
            None => Outcome::default(),
        }
    }

    /// Answers the waiting `Dialog` kind 10 with the Phonebook's choice:
    /// `Neighbor(Some(id))` for a call, `Neighbor(None)` for Cancel (stored as
    /// `-9999` by `end_of_update`, `0x4413e1`).
    pub fn execute_phonebook_command(&mut self, cmd: PhoneBookCommand) -> Outcome {
        let response = match cmd {
            PhoneBookCommand::Call(id) => SpecialPanelResponse::Neighbor(Some(id)),
            PhoneBookCommand::Cancel => SpecialPanelResponse::Neighbor(None),
        };
        self.phonebook_dialog.close();
        if self.answer_special_panel(response) {
            self.hud.play(crate::hud::CLICK);
        } else {
            self.hud.play(crate::hud::ERROR);
            if self.phonebook_snapshot.is_some() && self.active_special_panel.is_some() {
                self.phonebook_dialog.open();
            } else {
                self.phonebook_snapshot = None;
            }
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// The Camera Mode photo: the house view drawn alone (no panel, menus
    /// or dialogs), cropped by [`Game::capture_snapshot_canvas`] around the
    /// last Camera Mode click. The full frame is drawn again afterwards, so
    /// the canvas still shows the screen.
    pub fn take_photo(&mut self) -> Canvas {
        let at = self.photo_point.take();
        self.draw_frame(true);
        let shot = self.capture_snapshot_canvas(at);
        self.draw();
        shot
    }

    /// Crops the canvas to the Camera Mode frame for the active
    /// `SnapshotSize` (`0x4d1f6e`: [`sims_ui::camera::snapshot_capture_rect`]
    /// of the frame at the pointer, shifted into and clamped to the house
    /// viewport, less its outline). `at` is the clicked point; without one
    /// (the `C` screenshot key has no click) the pointer is the last one
    /// seen over the window, else the viewport's centre.
    pub fn capture_snapshot_canvas(&self, at: Option<[i32; 2]>) -> Canvas {
        let state = self.hud.camera_state();
        let size = state.size.dimensions(state.custom_dimensions);
        let src_w = self.canvas.width;
        let src_h = self.canvas.height;
        let (view_w, view_h) = self.house_viewport_size();
        let area_w = i32::try_from((view_w as usize).min(src_w)).unwrap_or(0);
        let area_h = i32::try_from((view_h as usize).min(src_h)).unwrap_or(0);
        if area_w <= 0 || area_h <= 0 {
            return self.canvas.clone();
        }
        let point = at.or(self.pointer).unwrap_or([area_w / 2, area_h / 2]);
        let [l, t, r, b] =
            sims_ui::camera::snapshot_capture_rect(point, size, [0, 0, area_w, area_h]);
        let crop_w = usize::try_from(r - l).unwrap_or(0);
        let crop_h = usize::try_from(b - t).unwrap_or(0);
        if crop_w == 0 || crop_h == 0 {
            // Native takes no picture of an empty frame (`0x4b055c`).
            return Canvas::new(0, 0, [0, 0, 0]);
        }
        let (x0, y0) = (l as usize, t as usize);
        let mut out = Canvas::new(crop_w, crop_h, [0, 0, 0]);
        for y in 0..crop_h {
            let src_row = (y0 + y) * src_w + x0;
            let dst_row = y * crop_w;
            out.color[dst_row..dst_row + crop_w]
                .copy_from_slice(&self.canvas.color[src_row..src_row + crop_w]);
        }
        out
    }
}
