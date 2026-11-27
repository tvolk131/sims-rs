//! Saving and restoring, the panel's commands, hotkeys, walls and floors.

use super::*;

impl Game {
    /// The lot as it stands, to go back to later ([`Game::restore`]).
    /// While a dialog waits, the save is of the lot before the tick that
    /// showed it, so the restored lot shows it again. While an unpaid Buy
    /// Mode preview is held, the preview and its `"Tile Cursor"` are stripped
    /// from the saved snapshot without disturbing the live placement.
    pub fn save(&self) -> Result<Saved, String> {
        match &self.buy_cursor {
            Some(cursor) => self
                .lot
                .save_without_buy_cursor(cursor)
                .map_err(|e| e.to_string()),
            None => self.lot.save().map_err(|e| e.to_string()),
        }
    }

    /// Keeps the lot in `store` under `slot`, saying across the top how
    /// that went. Returns `true` if the save succeeded (`0x4f26cf`, checked by
    /// `0x4f29cc` before exiting).
    pub fn save_to(&mut self, store: &sims_sim::Snapshots, slot: &str) -> bool {
        if self.coop_blocks_local_save() {
            self.note = Some("Cannot save a remote host's co-op lot to a local save slot.".into());
            return false;
        }
        let saved = self.save().and_then(|saved| {
            store
                .save_lot(slot, &saved.encode())
                .map_err(|e| e.to_string())
        });
        let ok = saved.is_ok();
        let followup = self.pending_after_save_note.take();
        self.note = Some(match saved {
            Ok(()) => {
                self.dirty = false;
                match followup {
                    Some(extra) => format!("Saved after tick {}. {extra}", self.lot.ticks()),
                    None => format!("Saved after tick {}.", self.lot.ticks()),
                }
            }
            Err(e) => format!("The lot was not saved: {e}"),
        });
        ok
    }

    /// Goes back to the lot saved in `store` under `slot`, saying across
    /// the top how that went. False if nothing changed.
    pub fn restore_from(&mut self, store: &sims_sim::Snapshots, slot: &str) -> bool {
        if self.coop.is_active_or_starting() {
            self.note = Some("Leave the co-op session before restoring a local save.".into());
            return false;
        }
        let saved = match store.load_lot(slot) {
            Ok(Some(bytes)) => Saved::decode(&bytes).map_err(|e| e.to_string()),
            Ok(None) => Err("nothing is saved for this house".into()),
            Err(e) => Err(e.to_string()),
        };
        let restored = saved.and_then(|saved| self.restore(saved));
        self.note = Some(match &restored {
            Ok(()) => format!("Back to the save after tick {}.", self.lot.ticks()),
            Err(e) => format!("The save was not restored: {e}"),
        });
        restored.is_ok()
    }

    /// Goes back to `saved`, a save of this house ([`Lot::restore`]). The
    /// lot then steps as the saved one would have. What the window kept
    /// of the lot before is dropped: why it stopped, the errors caught,
    /// the sounds not yet taken, an open or listed pie menu, the icons
    /// pressed and the dialog read. The panel's speed follows the saved
    /// lot's ([`pace::shown`]), the view turns to the saved lot's
    /// rotation, which the lot's code reads, and its people and rooms are
    /// read again. Sounds already playing are not stopped (**unverified** what a long one does once
    /// its object is gone).
    pub fn restore(&mut self, saved: Saved) -> Result<(), String> {
        self.lot.restore(saved).map_err(|e| e.to_string())?;
        self.world_mutated_via_restore = true;
        self.restore_generation = self.restore_generation.wrapping_add(1);
        self.clear_lot_cache();
        self.stopped = None;
        self.errors = Errors::default();
        self.sounds.clear();
        self.ui_effects.clear();
        self.highlighted_buttons.clear();
        self.highlighted_portraits.clear();
        self.highlighted_relationships.clear();
        self.tutorial_marker = None;
        self.marker_cell_drawn = None;
        self.reached = None;
        self.listed = None;
        self.highlighted = None;
        self.buy_selected = None;
        self.buy_cursor = None;
        self.buy_in_house = false;
        self.buy_last_guid = None;
        self.buy_facing = None;
        self.buy_dragging = None;
        self.buy_grab_offset = [0, 0];
        self.buy_commit_on_release = false;
        self.wall_cursor = None;
        self.build_drag = None;
        self.build_preview_shape = None;
        self.live_panel_expanded = true;
        self.submitted_coop_special_panel = None;
        self.close_special_panel_ui();
        self.pending_travel_lot = None;
        self.declined_trip = None;
        self.ui_animating_prev = false;
        self.hud.forget();
        self.hud.controls.mode = ui_mode(self.lot.mode());
        self.blink_dirty = false;
        self.dirty = false;
        self.restore_dirty_on_quit_cancel = false;
        self.pending_after_save_note = None;
        self.edge_scroll_rem = [0, 0];
        self.camera_pan = None;
        self.camera_pan_moved = false;
        // `0x517c21`, `0x46d8c9`: edit mode and the §999,999 edit budget both
        // read `[lot + 0x50]` (`global[32]`), so keep `edit_mode` synced with
        // the restored snapshot's global 32.
        self.edit_mode = self.lot.world().globals.get(32).copied().unwrap_or(0) == 1;
        // `0x4e66eb..0x4e6704 -> 0x432220`: restoring a lot keeps the player's
        // persisted Free Will preference and applies it to the restored lot.
        let free_will = self.hud.controls.options.play.free_will;
        if self.lot.world().globals.get(30).copied().unwrap_or(1) != i16::from(free_will) {
            let _ = self.lot.set_free_will(free_will);
        }
        self.hud.reset_character_names(self.lot.character_names());
        let world = self.lot.world();
        self.hud.sync_clock_blink(world, self.clock_us);
        if let Some(speed) = pace::shown(world) {
            self.pace.speed = speed;
        }
        // The view turns as the turn keys do, keeping the middle of the
        // screen, until it shows the lot as the saved lot was turned.
        let (w, h) = self.house_viewport_size();
        let viewport = [w as i32, h as i32];
        let rotation = self.lot.view_rotation();
        for _ in 0..4 {
            if self.view.camera.rotation == rotation {
                break;
            }
            if self.lot_shape.size > 0 {
                native_zoom_or_rotate(
                    &self.lot_shape,
                    &mut self.view.camera,
                    viewport,
                    Move::Rotate(true),
                );
                self.view.bound();
            } else {
                self.view
                    .apply_in(Move::Rotate(true), viewport, pick::ORIGIN_COLUMN);
            }
        }
        self.pace.restart();
        if !self.open_pack_mode {
            self.people.refresh(world);
        }
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
        self.hud.relayout_panel(
            self.view.size,
            self.lot.world(),
            self.lot.residential(),
            self.lot.lot_zoning(),
            self.lot_visibility(),
            &self.assets,
            None,
        );
        if self.hud.controls.mode == Mode::Build {
            self.layout_build();
        }
        Ok(())
    }

    /// Carries out a panel command. `menu` is the pie menu that was open
    /// when it was pressed.
    pub(super) fn command(&mut self, command: Command, menu: Option<&Menu>) -> Outcome {
        match command {
            Command::SetSpeed(speed) => {
                self.set_speed(speed);
                Outcome::redraw()
            }
            Command::TogglePause => {
                self.toggle_pause();
                Outcome::redraw()
            }
            Command::Camera(camera) => match camera {
                UiCamera::ZoomIn => {
                    self.apply(Move::ZoomIn);
                    Outcome::redraw()
                }
                UiCamera::ZoomOut => {
                    self.apply(Move::ZoomOut);
                    Outcome::redraw()
                }
                // Which way the original turns for each is unverified.
                UiCamera::RotateRight => {
                    self.apply(Move::Rotate(true));
                    Outcome::redraw()
                }
                UiCamera::RotateLeft => {
                    self.apply(Move::Rotate(false));
                    Outcome::redraw()
                }
                UiCamera::FloorUp => {
                    self.apply(Move::LevelUp);
                    Outcome::redraw()
                }
                UiCamera::FloorDown => {
                    self.apply(Move::LevelDown);
                    Outcome::redraw()
                }
                UiCamera::Walls(w) => {
                    self.apply(Move::Walls(match w {
                        UiWalls::Up => Walls::Up,
                        UiWalls::Cutaway => Walls::Cutaway,
                        UiWalls::Down => Walls::Down,
                    }));
                    Outcome::redraw()
                }
                UiCamera::Photo => {
                    self.toggle_camera_strip();
                    Outcome::redraw()
                }
                UiCamera::Roof => {
                    // Native `Sims.exe` `0x53a34d -> 0x4cef10(8, 1)`: 4-way
                    // mutually exclusive radio button (`ROOF` / `WALLS_UP` /
                    // `CUTAWAY` / `WALLS_DOWN`). Selecting Roof raises walls
                    // and enables roof rendering, unlighting all 3 wall buttons.
                    self.set_roof_view(true);
                    self.hud.play(crate::hud::CLICK);
                    Outcome::redraw()
                }
            },
            Command::ChooseInteraction {
                sim,
                object,
                interaction,
            } => {
                // `interaction` is the entry's index in the open menu.
                match menu.and_then(|m| m.entries.get(usize::from(interaction))) {
                    Some(entry) => self.order(sim, object, entry),
                    None => self.note = Some(format!("No menu entry {interaction} to choose.")),
                }
                Outcome::redraw()
            }
            Command::Select { sim } => {
                self.select(sim);
                Outcome::redraw()
            }
            Command::CancelAction { sim, action } => {
                self.cancel(sim, action);
                Outcome::redraw()
            }
            Command::AnswerDialog { caller, button } => {
                if caller == 0 && self.hud.has_ui_dialog() {
                    if let Some((kind, choice, entry, secondary)) =
                        self.hud.take_ui_dialog_answer(button)
                    {
                        self.hud.play(crate::hud::CLICK);
                        let apply_save_card = |lot: &mut Lot, dirty: &mut bool| {
                            let name = entry.unwrap_or_else(|| lot.lot_name().to_owned());
                            let desc =
                                secondary.unwrap_or_else(|| lot.lot_description().to_owned());
                            if name != lot.lot_name() || desc != lot.lot_description() {
                                *dirty = true;
                            }
                            lot.set_lot_info(name, desc);
                        };
                        match (kind, choice) {
                            (OptionsDialog::SaveCardSave, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmQuit, 0) => {
                                self.restore_dirty_on_quit_cancel = false;
                                return Outcome {
                                    redraw: true,
                                    quit: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmQuit, 1) => {
                                if std::mem::take(&mut self.restore_dirty_on_quit_cancel) {
                                    self.dirty = true;
                                }
                                self.note = None;
                            }
                            // `0x51681a..0x51686f`: edit-mode Save Card during
                            // Quit saves on Yes (`0x4f26cf`) and then enters
                            // the regular Quit confirmation (`0x4f290d`); on No
                            // it clears `dirty` temporarily for `ConfirmQuit`
                            // and restores `dirty` if the player declines.
                            (OptionsDialog::SaveCardQuit, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                self.restore_dirty_on_quit_cancel = false;
                                self.hud.open_options_dialog(
                                    OptionsDialog::ConfirmQuit,
                                    self.view.size,
                                    self.lot.lot_name(),
                                    self.lot.lot_description(),
                                );
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::SaveCardQuit, 1) => {
                                self.restore_dirty_on_quit_cancel = self.dirty;
                                self.dirty = false;
                                self.hud.open_options_dialog(
                                    OptionsDialog::ConfirmQuit,
                                    self.view.size,
                                    self.lot.lot_name(),
                                    self.lot.lot_description(),
                                );
                            }
                            (OptionsDialog::ConfirmSaveBeforeQuit, 0) => {
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    quit: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmSaveBeforeQuit, 1) => {
                                return Outcome {
                                    redraw: true,
                                    quit: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::SaveCardNeighborhood, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                let note = "Returning to Neighborhood.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    neighborhood: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmSaveBeforeNeighborhood, 0) => {
                                let note = "Returning to Neighborhood.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    neighborhood: true,
                                    ..Outcome::default()
                                };
                            }
                            (
                                OptionsDialog::SaveCardNeighborhood
                                | OptionsDialog::ConfirmSaveBeforeNeighborhood,
                                1,
                            ) => {
                                self.pending_after_save_note = None;
                                self.note = None;
                                return Outcome {
                                    redraw: true,
                                    neighborhood: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmResetTutorial, 0) => {
                                // `0x4f25ea -> 0x4f1a11..0x4f1ac6`: confirming
                                // Reset Tutorial on a dirty savable lot runs
                                // the native save confirmation before reset.
                                if self.edit_mode && self.dirty {
                                    self.hud.open_options_dialog(
                                        OptionsDialog::SaveCardResetTutorial,
                                        self.view.size,
                                        self.lot.lot_name(),
                                        self.lot.lot_description(),
                                    );
                                } else if self.dirty && self.lot_visibility().can_save {
                                    self.hud.open_options_dialog(
                                        OptionsDialog::ConfirmSaveBeforeResetTutorial,
                                        self.view.size,
                                        self.lot.lot_name(),
                                        self.lot.lot_description(),
                                    );
                                } else {
                                    self.note = Some("Tutorial reset is not ported yet.".into());
                                }
                            }
                            (OptionsDialog::SaveCardResetTutorial, 0) => {
                                apply_save_card(&mut self.lot, &mut self.dirty);
                                let note = "Tutorial reset is not ported yet.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (OptionsDialog::ConfirmSaveBeforeResetTutorial, 0) => {
                                let note = "Tutorial reset is not ported yet.";
                                self.pending_after_save_note = Some(note);
                                self.note = Some(note.into());
                                return Outcome {
                                    redraw: true,
                                    save: true,
                                    ..Outcome::default()
                                };
                            }
                            (
                                OptionsDialog::SaveCardResetTutorial
                                | OptionsDialog::ConfirmSaveBeforeResetTutorial,
                                1,
                            ) => {
                                self.pending_after_save_note = None;
                                self.note = Some("Tutorial reset is not ported yet.".into());
                            }
                            (OptionsDialog::SpecialBook, _) => {
                                if !self.answer_special_panel(SpecialPanelResponse::Dismissed) {
                                    // Restore the book modal if co-op submission failed so
                                    // the blocking panel never becomes invisible.
                                    self.active_special_panel = None;
                                    self.sync_special_panel();
                                }
                            }
                            (OptionsDialog::Notice, _) => {
                                self.note = None;
                                if let Some(next) = self.pending_after_notice.take() {
                                    let name = self.lot.lot_name().to_owned();
                                    let desc = self.lot.lot_description().to_owned();
                                    self.hud.open_options_dialog(
                                        next,
                                        self.view.size,
                                        &name,
                                        &desc,
                                    );
                                }
                            }
                            _ => {
                                self.note = None;
                            }
                        }
                    }
                } else {
                    self.answer(caller, button);
                }
                Outcome::redraw()
            }
            Command::OptionsAction(action) => {
                if self.coop.is_peer()
                    && matches!(action, OptionsAction::Save | OptionsAction::Neighborhood)
                {
                    self.note = Some(
                        "Only the host can save or return to Neighborhood; use Co-op -> Leave to disconnect."
                            .into(),
                    );
                    return Outcome::redraw();
                }
                let can_save = self.lot_visibility().can_save;
                match action {
                    // `0x516874`: calls `0x517c03(0)` (shows the save card only
                    // when `[lot + 0x50] == 1` edit mode at `0x517c21`) and
                    // otherwise proceeds directly to `0x4f26cf` (save).
                    OptionsAction::Save if !self.edit_mode => {
                        self.pending_after_save_note = None;
                        Outcome {
                            redraw: true,
                            save: true,
                            ..Outcome::default()
                        }
                    }
                    OptionsAction::Save => {
                        self.hud.open_options_dialog(
                            OptionsDialog::SaveCardSave,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                    // `0x516809..0x51686f` -> `0x4f290d`:
                    // - edit mode & dirty -> save card (`0x517c03(1)`)
                    // - ordinary dirty & savable -> 3-button `STR# 153[5..6]` (`0x4f298e`)
                    // - clean or non-savable -> 2-button `STR# 153[1..2]` (`0x4f29f1`/`0x4f2a46`)
                    OptionsAction::Quit => {
                        let dialog = if self.edit_mode && self.dirty {
                            OptionsDialog::SaveCardQuit
                        } else if self.dirty && can_save {
                            OptionsDialog::ConfirmSaveBeforeQuit
                        } else {
                            OptionsDialog::ConfirmQuit
                        };
                        self.hud.open_options_dialog(
                            dialog,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                    // `0x5170f0..0x51713c`: prompts only when `dirty && can_save`;
                    // edit mode uses the save card (`0x517c03(1)`), ordinary lots
                    // fall back to `0x4f1a11` (`STR# 153[3..4]`).
                    OptionsAction::Neighborhood if !self.dirty || !can_save => Outcome {
                        redraw: true,
                        neighborhood: true,
                        ..Outcome::default()
                    },
                    OptionsAction::Neighborhood => {
                        let dialog = if self.edit_mode {
                            OptionsDialog::SaveCardNeighborhood
                        } else {
                            OptionsDialog::ConfirmSaveBeforeNeighborhood
                        };
                        self.hud.open_options_dialog(
                            dialog,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                    // `0x4f25b4`: 2-button `STR# 153[7..8]` confirmation.
                    OptionsAction::TutorialReset => {
                        self.hud.open_options_dialog(
                            OptionsDialog::ConfirmResetTutorial,
                            self.view.size,
                            self.lot.lot_name(),
                            self.lot.lot_description(),
                        );
                        Outcome::redraw()
                    }
                }
            }
            Command::ToggleOption(toggle) => {
                if toggle == OptionsToggle::FreeWill {
                    // `0x432220..0x432242` -> `0x46d9c4(0x1e, al)`: write global 30.
                    let enabled = self.hud.controls.options.play.free_will;
                    if self.coop.is_active() {
                        if let Err(e) = self
                            .coop
                            .submit_command(NetCommand::SetFreeWill { enabled })
                        {
                            self.note = Some(format!("Free Will was not changed: {e}"));
                        }
                    } else if let Err(e) = self.lot.set_free_will(enabled) {
                        self.note = Some(format!("Free Will was not changed: {e}"));
                    } else {
                        self.dirty = true;
                    }
                }
                Outcome::redraw()
            }
            Command::OpenHelp => {
                self.note = Some("The Help System dialog is not ported yet.".into());
                Outcome::redraw()
            }
            Command::SetMode(mode) => {
                self.set_mode(mode);
                Outcome::redraw()
            }
            Command::BuyCategory(_) | Command::BuySubcategory(_) | Command::BuyBack => {
                self.cancel_buy();
                self.layout_buy();
                Outcome::redraw()
            }
            Command::BuyToggleSort => {
                self.toggle_buy_sort();
                Outcome::redraw()
            }
            Command::BuyPage(_) => {
                self.layout_buy();
                Outcome::redraw()
            }
            Command::BuyItem(guid) => {
                self.select_buy_item(guid);
                Outcome::redraw()
            }
            Command::BuildTool(tool) => {
                self.select_build_tool(tool);
                Outcome::redraw()
            }
            Command::BuildPage(delta) => {
                self.page_build(delta);
                Outcome::redraw()
            }
            Command::BuildItem(item) => {
                self.select_build_item(item);
                Outcome::redraw()
            }
            Command::Undo => {
                self.undo_buy();
                Outcome::redraw()
            }
            Command::Redo => {
                self.redo_buy();
                Outcome::redraw()
            }
            Command::TogglePanel => {
                self.hud.close_camera_strip();
                if self.hud.controls.mode == Mode::Live {
                    self.live_panel_expanded = self.hud.controls.expanded;
                }
                self.sync_view_centre();
                if self.hud.controls.mode == Mode::Buy {
                    self.layout_buy();
                } else if self.hud.controls.mode == Mode::Build {
                    self.layout_build();
                }
                Outcome::redraw()
            }
            Command::OpenOptionsTab(_) | Command::OptionsHelp(_) => {
                self.hud.relayout_panel(
                    self.view.size,
                    self.lot.world(),
                    self.lot.residential(),
                    self.lot.lot_zoning(),
                    self.lot_visibility(),
                    &self.assets,
                    None,
                );
                Outcome::redraw()
            }
            Command::OpenPanel(_)
            | Command::Refused { .. }
            | Command::ShowRelations(_)
            | Command::RelationsPage(_)
            | Command::InterestsPage(_)
            | Command::ShowInventory(_)
            | Command::InventoryPage(_)
            | Command::SelectInventoryItem(_)
            | Command::Help(_)
            | Command::SetDetail { .. }
            | Command::SetVolume { .. } => Outcome::redraw(),
        }
    }

    /// The active control-panel mode (`Mode::Live`, `Mode::Buy`, etc.).
    pub fn mode(&self) -> Mode {
        self.hud.controls.mode
    }

    /// The active simulation speed (`Speed::Normal`, `Speed::Fast`, `Speed::Ultra`).
    pub fn speed(&self) -> Speed {
        self.pace.speed
    }

    /// Whether the bottom control panel strip is currently expanded.
    pub fn panel_expanded(&self) -> bool {
        self.hud.controls.expanded
    }

    /// Active wall display mode (`Walls::Up`, `Walls::Cutaway`, `Walls::Down`).
    pub fn walls(&self) -> Walls {
        self.view.camera.walls
    }

    /// Active displayed floor level (`1` = ground, `2` = upper).
    pub fn level(&self) -> i32 {
        self.view.camera.level
    }

    pub(super) fn flush_hud_sounds(&mut self) {
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
    }

    /// Toggles the bottom control panel strip collapsed/expanded (`F10` or
    /// pressing `F1` while already in Live Mode, `0x4cfc75`). Closes the Camera
    /// strip if open, clears any open help balloons (`Command::TogglePanel`),
    /// and re-centers the active house viewport. Defers while a Buy or Build
    /// pointer drag is in progress so expanding the strip underneath a held
    /// object cannot turn a move release into an unintended sale.
    pub fn toggle_panel(&mut self) -> bool {
        if self.buy_dragging.is_some() || self.buy_commit_on_release || self.build_drag.is_some() {
            return false;
        }
        self.hud.close_camera_strip();
        let had_balloon =
            self.hud.controls.help.is_some() || self.hud.controls.options.help.is_some();
        self.hud.controls.apply(&Command::TogglePanel);
        if self.hud.controls.mode == Mode::Live {
            self.live_panel_expanded = self.hud.controls.expanded;
        }
        self.sync_view_centre();
        if self.hud.controls.mode == Mode::Buy {
            self.layout_buy();
        } else if self.hud.controls.mode == Mode::Build {
            self.layout_build();
        }
        // `TogglePanel` drops any help balloon: it fades out and whooshes.
        self.hud.balloon_whoosh(had_balloon);
        self.hud.play(crate::hud::CLICK);
        self.flush_hud_sounds();
        true
    }

    /// Toggles the Camera / Photo Mode strip (`F4`, `0x4d1712` / `0x53a543`).
    /// Defers while a Buy or Build pointer drag is in progress.
    pub fn toggle_camera_strip(&mut self) -> bool {
        if self.buy_dragging.is_some() || self.buy_commit_on_release || self.build_drag.is_some() {
            return false;
        }
        let was_expanded = self.hud.controls.expanded;
        self.hud.toggle_camera_strip(self.view.size);
        if self.hud.controls.expanded != was_expanded {
            if self.hud.controls.mode == Mode::Live {
                self.live_panel_expanded = self.hud.controls.expanded;
            }
            self.sync_view_centre();
            if self.hud.controls.mode == Mode::Buy {
                self.layout_buy();
            } else if self.hud.controls.mode == Mode::Build {
                self.layout_build();
            }
        }
        self.hud.play(crate::hud::CLICK);
        self.flush_hud_sounds();
        true
    }

    /// Authentic `Sims.exe` 1.91 `F1`/`F2`/`F3`/`F5` mode hotkey dispatch
    /// (`0x4d16e2..0x4d1869`):
    /// - `F1` (`Mode::Live`, `0x4d1826`): enters Live Mode, closes Camera strip
    ///   if open, or toggles panel collapse/expand when already in Live Mode.
    /// - `F2` (`Mode::Buy`, `0x4d1787`): expands/enters Buy Mode, or toggles
    ///   `ByFunction` ↔ `ByRoom` sort when already in Buy Mode (`0x4cf3af`).
    /// - `F3` (`Mode::Build`, `0x4d1750`): expands and enters Build Mode.
    /// - `F5` (`Mode::Options`, `0x4d16e2`): expands and enters Options Mode.
    pub fn hotkey_mode(&mut self, mode: Mode) {
        match mode {
            Mode::Live => {
                if self.hud.controls.mode == Mode::Live {
                    if self.hud.camera_strip_open() {
                        self.hud.close_camera_strip();
                        self.hud.play(crate::hud::CLICK);
                        self.flush_hud_sounds();
                    } else {
                        self.toggle_panel();
                    }
                } else {
                    self.set_mode(Mode::Live);
                }
            }
            Mode::Buy => {
                if self.hud.controls.mode == Mode::Buy && !self.hud.camera_strip_open() {
                    if !self.hud.controls.expanded {
                        self.toggle_panel();
                    } else if self.toggle_buy_sort() {
                        self.hud.play(crate::hud::CLICK);
                        self.flush_hud_sounds();
                    }
                } else {
                    self.set_mode(Mode::Buy);
                }
            }
            Mode::Build => {
                if self.hud.controls.mode == Mode::Build && !self.hud.camera_strip_open() {
                    if !self.hud.controls.expanded {
                        self.toggle_panel();
                    }
                } else {
                    self.set_mode(Mode::Build);
                }
            }
            Mode::Options => {
                self.set_mode(Mode::Options);
            }
        }
    }

    /// Cycles selection to the next Sim in the household roster, wrapping
    /// around (`Space` in Live Mode, `cWinLive::OnKeyDown` at `0x51ba2d..0x51ba7c`).
    /// Only active in Live Mode so Buy/Build/Options editing never switches
    /// household selection. Closes any open pie menu and plays `UI_click`. If
    /// only one Sim is in the household, centres the camera on that Sim.
    pub fn select_next_sim(&mut self) -> bool {
        if self.hud.controls.mode != Mode::Live {
            return false;
        }
        let residents = self.hud.resident_object_ids(self.lot.world());
        if residents.is_empty() {
            return false;
        }
        let world = self.lot.world();
        let current = self.local_selected_sim();
        let current_guid = current
            .and_then(|id| world.objects.get(&id))
            .map(|o| o.guid);
        let current_idx = residents.iter().position(|&id| {
            current == Some(id)
                || current_guid.is_some() && world.objects.get(&id).map(|o| o.guid) == current_guid
        });
        let next = match current_idx {
            Some(idx) => residents[(idx + 1) % residents.len()],
            None => residents[0],
        };
        self.close_menu();
        self.select(next);
        self.flush_hud_sounds();
        true
    }

    /// Centres the camera on the currently selected Sim (`Enter` in
    /// `OnKeyDown`, `0x4d1330 -> 0x4d14d2..0x4d1507`).
    pub fn centre_on_selected(&mut self) -> bool {
        let Some(sim) = self.local_selected_sim() else {
            return false;
        };
        let world = self.lot.world();
        let target = world
            .objects
            .get(&sim)
            .and_then(|o| sims_ui::household::first_person_for_guid(world, o.guid))
            .map_or(sim, |o| o.id);
        let smooth = self.smooth_pan_active();
        self.centre_on_with_pan(target, smooth);
        true
    }

    /// Steps wall display mode upward (`Home`: `Down → Cutaway → Up`, `0x4d135b`)
    /// or downward (`End`: `Up → Cutaway → Down`, `0x4d1393`), playing
    /// `UI_error` at either boundary. When Roof view (`[ui + 0x48] == 8`) is
    /// active, `Home`/`End` do not match `1`/`2`/`4` in `Sims.exe` 1.91 and
    /// leave Roof view unchanged without playing an error.
    pub fn step_walls(&mut self, up: bool) -> bool {
        if self.roof_view {
            return false;
        }
        let next = match (self.view.camera.walls, up) {
            (Walls::Down, true) => Some(Walls::Cutaway),
            (Walls::Cutaway, true) => Some(Walls::Up),
            (Walls::Up, true) => None,
            (Walls::Up, false) => Some(Walls::Cutaway),
            (Walls::Cutaway, false) => Some(Walls::Down),
            (Walls::Down, false) => None,
        };
        match next {
            Some(walls) => self.apply(Move::Walls(walls)),
            None => {
                self.hud.play(crate::hud::ERROR);
                self.flush_hud_sounds();
                false
            }
        }
    }

    /// Toggles between ground floor (`1`) and upper floor (`2`) (`Tab` in
    /// `OnKeyDown`, `0x4d1324 -> 0x4d150c`).
    pub fn toggle_floor(&mut self) -> bool {
        if self.view.camera.level <= 1 {
            self.apply(Move::LevelUp)
        } else {
            self.apply(Move::LevelDown)
        }
    }
}
