//! Neighborhood and family bin commands, lots and the shell's dialogs.

use super::*;

impl NeighborhoodShell {
    pub(super) fn handle_neighborhood_command(&mut self, cmd: NeighborhoodCommand) -> ShellOutcome {
        match cmd {
            NeighborhoodCommand::PreviousNeighborhood => {
                let prev = self.nbhd_state.previous_neighborhood();
                if let Err(e) = self.switch_neighborhood(prev) {
                    self.note = Some(format!("Could not switch neighborhood: {e}"));
                }
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::NextNeighborhood => {
                let next = self.nbhd_state.next_neighborhood();
                if let Err(e) = self.switch_neighborhood(next) {
                    self.note = Some(format!("Could not switch neighborhood: {e}"));
                }
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenFamilyBin => {
                self.open_family_bin();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::CancelMoveIn => {
                self.nbhd_state.mode = NeighborhoodMode::Normal;
                self.move_in_family_id = None;
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::ToggleBulldoze => {
                self.nbhd_state.mode = if matches!(self.nbhd_state.mode, NeighborhoodMode::Bulldoze)
                {
                    NeighborhoodMode::Normal
                } else {
                    self.move_in_family_id = None;
                    NeighborhoodMode::Bulldoze
                };
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenDowntown => {
                self.toggle_sub_neighborhood(NeighborhoodKind::Downtown);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenVacationIsland => {
                self.toggle_sub_neighborhood(NeighborhoodKind::Vacation);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenStudioTown => {
                self.toggle_sub_neighborhood(NeighborhoodKind::StudioTown);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenMagicTown => {
                self.toggle_sub_neighborhood(NeighborhoodKind::MagicTown);
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::Quit => {
                self.open_quit_dialog();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::ToggleRezone => {
                self.nbhd_state.mode = if matches!(self.nbhd_state.mode, NeighborhoodMode::Rezone) {
                    NeighborhoodMode::Normal
                } else {
                    self.move_in_family_id = None;
                    NeighborhoodMode::Rezone
                };
                self.refresh_hovered_lot();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::ImportFamily => {
                self.open_import_family_picker();
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenWeb => {
                let title = self
                    .chrome_strings
                    .bar
                    .get(sims_ui::neighborhood::BarButton::Web.tooltip_index())
                    .filter(|s| !s.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| "Web".into());
                self.open_dialog(
                    ShellDialogKind::Notice,
                    Message::notice(
                        title,
                        "This offline engine does not launch external web browsers.",
                        self.dialogs.ok(),
                    )
                    .with_frame(FrameStyle::Generic),
                );
                ShellOutcome::redraw()
            }
            NeighborhoodCommand::OpenCredits => {
                self.open_credits_with_music();
                ShellOutcome::redraw()
            }
        }
    }

    pub(super) fn toggle_sub_neighborhood(&mut self, target: NeighborhoodKind) {
        let next = if self.kind == target {
            default_map_kind(&self.install, self.slot)
        } else {
            target
        };
        if let Err(e) = self.switch_kind(next) {
            self.note = Some(format!("Could not open map view: {e}"));
        }
    }

    pub(super) fn handle_family_bin_command(&mut self, cmd: FamilyBinCommand) -> ShellOutcome {
        match cmd {
            FamilyBinCommand::SelectFamily(_) => ShellOutcome::redraw(),
            FamilyBinCommand::Cancel => {
                self.close_family_bin();
                ShellOutcome::redraw()
            }
            FamilyBinCommand::MoveInFamily(id) => {
                if let Ok(fam_id) = u16::try_from(id)
                    && let Err(e) = self.start_move_in(fam_id)
                {
                    self.note = Some(e);
                }
                ShellOutcome::redraw()
            }
            FamilyBinCommand::DeleteFamily(id) => {
                if let Ok(fam_id) = u16::try_from(id) {
                    let name = self
                        .family_names
                        .get(&fam_id)
                        .cloned()
                        .unwrap_or_else(|| format!("Family {fam_id}"));
                    let title = DialogStrings::get(&self.dialogs.family_bin.entries, 6, "Delete");
                    let tmpl = DialogStrings::get(&self.dialogs.family_bin.entries, 7, "%s");
                    let body = format_template(tmpl, &[&name]);
                    self.open_dialog(
                        ShellDialogKind::ConfirmDeleteFamily { family_id: fam_id },
                        Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                            .with_frame(FrameStyle::Generic),
                    );
                }
                ShellOutcome::redraw()
            }
            FamilyBinCommand::CreateNewFamily => {
                if let Err(err) = self.open_create_family() {
                    self.note = Some(format!("Could not open Create-a-Family: {err}"));
                }
                ShellOutcome::redraw()
            }
        }
    }

    pub(super) fn activate_lot(&mut self, lot_id: u16) -> ShellOutcome {
        match self.nbhd_state.mode.clone() {
            NeighborhoodMode::Rezone => self.prompt_rezone_lot(lot_id),
            NeighborhoodMode::Normal | NeighborhoodMode::Visit => {
                let house = u32::from(lot_id);
                if let Ok(Some(trip)) =
                    vm_travel::inspect_active_trip(&self.store, &self.neighborhood_id, house)
                {
                    let title = format!("Active Travel Trip (Lot {house})");
                    let body = format!(
                        "This household has an interrupted trip at destination lot {}. Resume the trip or abort and restore the frozen home lot?",
                        trip.destination_house
                    );
                    self.open_dialog(
                        ShellDialogKind::ConfirmResumeOrAbortTrip {
                            home_house: house,
                            destination_house: trip.destination_house,
                        },
                        Message::confirm(
                            title,
                            body,
                            vec!["Resume Trip".into(), "Abort Trip".into(), "Cancel".into()],
                        )
                        .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let is_unoccupied_destination = destination_for_kind(self.kind).is_some()
                    && TravelDestination::classify(house, &self.stored.zoning).is_some()
                    && self.stored.family_in(house).is_none();
                let res = if is_unoccupied_destination {
                    self.open_destination_for_edit(house)
                } else {
                    self.enter_lot(house)
                };
                if let Err(e) = res {
                    // Non-destructive corrupt-save fallback: if intact autosaves
                    // exist for this lot, open the Save Manager with the error banner.
                    if self
                        .store
                        .autosaves()
                        .list_saves_metadata(&self.neighborhood_id, house)
                        .is_ok_and(|slots| !slots.is_empty())
                    {
                        self.save_manager_target_lot = Some(house);
                        self.open_save_manager(Some(format!("Could not open lot {lot_id}: {e}")));
                    } else {
                        self.note = Some(format!("Could not enter lot {lot_id}: {e}"));
                    }
                }
                ShellOutcome::redraw()
            }
            NeighborhoodMode::MoveIn {
                family_funds,
                family_magic_coins,
                ..
            } => {
                let Some(family_id) = self.move_in_family_id else {
                    self.nbhd_state.mode = NeighborhoodMode::Normal;
                    return ShellOutcome::redraw();
                };
                let is_res = self.stored.is_residential(u32::from(lot_id));
                if !is_res {
                    let title = DialogStrings::get(&self.dialogs.move_in, 12, "Community");
                    let body = DialogStrings::get(&self.dialogs.move_in, 13, "Not residential.");
                    self.open_dialog(
                        ShellDialogKind::Notice,
                        Message::notice(title, body, self.dialogs.ok())
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                if self.stored.family_in(u32::from(lot_id)).is_some() {
                    let title = DialogStrings::get(&self.dialogs.move_in, 14, "Occupied");
                    let body = DialogStrings::get(&self.dialogs.move_in, 15, "Lot is occupied.");
                    self.open_dialog(
                        ShellDialogKind::Notice,
                        Message::notice(title, body, self.dialogs.ok())
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let meta = self.lot_meta.get(&lot_id);
                let cost = meta.map_or(0, |m| m.house_cost);
                let magic_cost = self.magic_cost_for_lot(lot_id);
                let has_house = meta.is_some_and(|m| m.has_house);
                if family_funds < cost || (magic_cost > 0 && family_magic_coins < magic_cost) {
                    // Hover card already explains insufficient Simoleons or MagiCoins.
                    return ShellOutcome::redraw();
                }
                let (title_idx, body_idx) = if has_house { (0, 1) } else { (6, 7) };
                let title = DialogStrings::get(&self.dialogs.move_in, title_idx, "Move In");
                let body = DialogStrings::get(&self.dialogs.move_in, body_idx, "Move in?");
                self.open_dialog(
                    ShellDialogKind::ConfirmMoveIn { family_id, lot_id },
                    Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                        .with_frame(FrameStyle::Generic),
                );
                ShellOutcome::redraw()
            }
            NeighborhoodMode::Bulldoze => {
                if let Some((fam_id, fam)) = self.stored.family_in(u32::from(lot_id)) {
                    let name = self
                        .family_names
                        .get(&fam_id)
                        .cloned()
                        .unwrap_or_else(|| format!("Family {fam_id}"));
                    let worth = format_simoleons(family_net_worth(fam));
                    let title = DialogStrings::get(&self.dialogs.bulldoze, 2, "Evict");
                    let tmpl = DialogStrings::get(&self.dialogs.bulldoze, 3, "%s (%s)");
                    let body = format_template(tmpl, &[&name, &worth]);
                    self.open_dialog(
                        ShellDialogKind::ConfirmEvict { lot_id },
                        Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let has_house = self.lot_meta.get(&lot_id).is_some_and(|m| m.has_house);
                if !has_house {
                    let title = DialogStrings::get(&self.dialogs.bulldoze, 4, "Bulldoze");
                    let body = DialogStrings::get(&self.dialogs.bulldoze, 5, "Empty lot.");
                    self.open_dialog(
                        ShellDialogKind::Notice,
                        Message::notice(title, body, self.dialogs.ok())
                            .with_frame(FrameStyle::Generic),
                    );
                    return ShellOutcome::redraw();
                }
                let title = DialogStrings::get(&self.dialogs.bulldoze, 0, "Bulldoze");
                let body = DialogStrings::get(&self.dialogs.bulldoze, 1, "Bulldoze?");
                self.open_dialog(
                    ShellDialogKind::ConfirmBulldoze { lot_id },
                    Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                        .with_frame(FrameStyle::Generic),
                );
                ShellOutcome::redraw()
            }
        }
    }

    pub(super) fn open_quit_dialog(&mut self) {
        let (title, body) = (
            self.dialogs.options.confirm_quit.0.clone(),
            self.dialogs.options.confirm_quit.1.clone(),
        );
        self.open_dialog(
            ShellDialogKind::ConfirmQuit,
            Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                .with_frame(FrameStyle::Generic),
        );
    }

    pub(super) fn clear_hover_state(&mut self) {
        self.hovered_lot_id = None;
        self.hovered_lot_since_us = None;
        self.hover_card_shown_for = None;
        self.nbhd_state.hovered_lot = None;
        self.hover_fade.reset();
        self.hover_fade_was_animating = false;
    }

    pub(super) fn open_dialog(&mut self, kind: ShellDialogKind, message: Message) {
        self.clear_hover_state();
        let state = DialogState::for_message(&message);
        let layout = dialog::layout(self.size, &message, &self.dialog_art);
        self.dialog = Some(ShellDialog {
            kind,
            message,
            state,
            layout,
        });
        if self.clock_us > 0 {
            let now_ms = (self.clock_us / 1_000) as u32;
            self.dialog_fade = DialogFadeState::hidden();
            self.dialog_fade
                .fade_to(1.0, now_ms, DIALOG_FADE_DURATION_MS);
            self.dialog_fade_was_animating = true;
        } else {
            self.dialog_fade = DialogFadeState::opaque();
            self.dialog_fade_was_animating = false;
        }
    }

    pub(super) fn relayout_dialog(&mut self) {
        if let Some(dlg) = &mut self.dialog {
            dlg.layout = dialog::layout(self.size, &dlg.message, &self.dialog_art);
        }
    }

    pub(super) fn answer_dialog(&mut self, button: u8) -> ShellOutcome {
        let Some(dlg) = self.dialog.take() else {
            return ShellOutcome::default();
        };
        self.dialog_fade = DialogFadeState::hidden();
        self.dialog_fade_was_animating = false;
        if let ShellDialogKind::ConfirmResumeOrAbortTrip {
            home_house,
            destination_house,
        } = dlg.kind
        {
            match button {
                0 => {
                    if let Err(e) = self.resume_active_trip(home_house) {
                        self.note = Some(format!(
                            "Could not resume trip to lot {destination_house}: {e}"
                        ));
                    }
                }
                1 => match self.abort_active_trip(home_house) {
                    Ok(restored) => {
                        self.note = Some(format!(
                            "Aborted interrupted trip and restored home lot {restored}."
                        ));
                    }
                    Err(e) => {
                        self.note = Some(format!("Could not abort interrupted trip: {e}"));
                    }
                },
                _ => {}
            }
            return ShellOutcome::redraw();
        }
        if button != 0 {
            return ShellOutcome::redraw();
        }
        match dlg.kind {
            ShellDialogKind::ConfirmQuit => ShellOutcome {
                redraw: true,
                quit: true,
                photo: false,
            },
            ShellDialogKind::ConfirmMoveIn { family_id, lot_id } => {
                if let Err(e) = self.execute_move_in(family_id, u32::from(lot_id)) {
                    self.note = Some(format!("Move-In failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmEvict { lot_id } => {
                if let Err(e) = self.execute_evict(u32::from(lot_id)) {
                    self.note = Some(format!("Evict failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmBulldoze { lot_id } => {
                if let Err(e) = self.execute_bulldoze(u32::from(lot_id)) {
                    self.note = Some(format!("Bulldoze failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmDeleteFamily { family_id } => {
                if let Err(e) = self.execute_delete_family(family_id) {
                    self.note = Some(format!("Delete family failed: {e}"));
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmRezone {
                lot_id,
                target_zone,
                expected_instance_id,
                expected_revision,
                expected_occupant,
            } => {
                match self.execute_rezone_checked(
                    lot_id,
                    target_zone,
                    Some((expected_instance_id, expected_revision, expected_occupant)),
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        let _ = self.refresh_from_store();
                        self.open_dialog(
                            ShellDialogKind::Notice,
                            Message::notice(
                                DialogStrings::get(&self.dialogs.rezone, 0, "Rezone"),
                                format!("Could not rezone lot {lot_id}: {e}"),
                                self.dialogs.ok(),
                            )
                            .with_frame(FrameStyle::Generic),
                        );
                    }
                }
                ShellOutcome::redraw()
            }
            ShellDialogKind::ConfirmResumeOrAbortTrip { .. } => unreachable!(),
            ShellDialogKind::Notice => ShellOutcome::redraw(),
        }
    }
}
