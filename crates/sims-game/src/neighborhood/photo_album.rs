//! The photo album and family storybook (`sims_ui::album`).

use super::*;

impl NeighborhoodShell {
    // =========================================================================
    // Photo Album / Family Storybook Modal (`sims_ui::album` + `PhotoAlbumStore`)
    // =========================================================================

    /// Opens the Photo Album modal for the active lot's household (or hovered /
    /// first neighborhood household when on the Neighborhood map).
    pub fn open_photo_album(&mut self) -> Result<(), String> {
        let family_id = self
            .album_family()
            .ok_or_else(|| "no household available for Photo Album".to_owned())?;
        self.open_photo_album_for_family(family_id)
    }

    /// The household whose album the Photo Album opens and Camera Mode
    /// photos go into: the active lot's family, or the hovered or first
    /// household on the Neighborhood map.
    pub(super) fn album_family(&self) -> Option<u16> {
        match &self.screen {
            ActiveScreen::Lot(session) => {
                if let Some(trip) = &session.active_trip {
                    Some(trip.home_family)
                } else {
                    self.stored
                        .family_in(session.house)
                        .map(|(id, _)| id)
                        .or_else(|| {
                            u16::try_from(
                                session
                                    .game
                                    .lot()
                                    .world()
                                    .globals
                                    .get(9)
                                    .copied()
                                    .unwrap_or(0),
                            )
                            .ok()
                            .filter(|&id| id != 0)
                        })
                }
            }
            _ => self
                .hovered_lot_id
                .and_then(|lot| self.stored.family_in(u32::from(lot)).map(|(id, _)| id))
                .or_else(|| self.stored.families.keys().next().copied()),
        }
    }

    /// Opens the Photo Album modal for a specific `family_id` in the active
    /// neighborhood, acquiring a fenced [`AlbumFamilyToken`] from REDB.
    pub fn open_photo_album_for_family(&mut self, family_id: u16) -> Result<(), String> {
        let nh_id = match &self.screen {
            ActiveScreen::Lot(session) => session.neighborhood_id.clone(),
            _ => self.neighborhood_id.clone(),
        };
        let token = self
            .store
            .albums()
            .family_token(&nh_id, family_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("family {family_id} not found in neighborhood {nh_id:?}"))?;
        // A fresh dialog, as native's new `cWinScrapbook` is (see
        // `PhotoAlbumDialog::reset_view`), on the album's current photo.
        self.album_dialog.reset_view();
        let (snapshot, has_image) = self.build_album_snapshot(&nh_id, family_id, None)?;
        self.clear_hover_state();
        if let ActiveScreen::Lot(session) = &mut self.screen {
            session.game.close_camera_strip();
        }
        self.album_token = Some(token);
        self.album_has_image = has_image;
        self.album_dialog.open(&snapshot);
        // `0x52b119` shows the album's current photo; Done leaves it as it
        // was last shown (`close_photo_album`).
        let current = self.album_index.get(&token).copied().unwrap_or(0);
        self.album_dialog.show_index(&snapshot, current);
        self.album_neighborhood_id = Some(nh_id);
        self.album_snapshot = Some(snapshot);
        self.hydrate_album_images();
        Ok(())
    }

    /// Stores a Camera Mode photo of the open lot in its household's album
    /// (`cWinCamera`: clicking the lot takes the picture). The picture is
    /// the Camera Mode size crop of the house view ([`Game::take_photo`]).
    ///
    /// An error means the photo was not stored. Once it is stored the ID
    /// is returned; an open album that then fails to refresh is reported
    /// with a notice, not as an error, so a retry cannot store it twice.
    pub fn capture_lot_photo(&mut self) -> Result<PhotoId, String> {
        let family_id = self
            .album_family()
            .ok_or_else(|| "no household for the photo".to_owned())?;
        let ActiveScreen::Lot(session) = &mut self.screen else {
            return Err("a photo needs an open lot".to_owned());
        };
        let nh_id = session.neighborhood_id.clone();
        let image = capture_canvas_to_album_image(&session.game.take_photo())?;
        let sim_day = u32::try_from(
            session
                .game
                .lot()
                .world()
                .globals
                .get(28)
                .copied()
                .unwrap_or(0)
                .max(0),
        )
        .ok()
        .map(|d| d + 1);
        let title = {
            let name = session.game.lot().lot_name().trim();
            if name.is_empty() {
                format!("Lot {}", session.house)
            } else {
                name.to_owned()
            }
        };
        let token = self
            .store
            .albums()
            .family_token(&nh_id, family_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("family {family_id} not found in neighborhood {nh_id:?}"))?;
        let mut entry = NewAlbumPhoto::with_image(title, "", image);
        if let Some(day) = sim_day {
            entry = entry.with_sim_day(day);
        }
        let photo = self
            .store
            .albums()
            .add(&nh_id, token, entry)
            .map_err(|e| e.to_string())?;
        let id = PhotoId(photo.id);
        // Adding a photo makes it the album's current one (`0x4ed857`): the
        // index past the end is clamped to the newest when the album opens.
        // An album open now keeps the photo it shows, and records that on
        // close.
        self.album_index.insert(token, usize::MAX);
        if self.album_dialog.is_open()
            && self.album_token == Some(token)
            && let Err(err) = self.refresh_album_snapshot()
        {
            self.show_notice(
                "Photo Album",
                format!("The photo was saved, but the album did not refresh: {err}"),
            );
        }
        Ok(id)
    }

    /// Closes the Photo Album modal cleanly. The album keeps the photo it
    /// showed as its current one, for the next opening.
    pub fn close_photo_album(&mut self) {
        if self.album_dialog.is_open()
            && let (Some(token), Some(snapshot)) = (self.album_token, self.album_snapshot.as_ref())
        {
            let shown = self.album_dialog.shown_index(snapshot).unwrap_or(0);
            self.album_index.insert(token, shown);
        }
        self.album_dialog.close();
        self.album_snapshot = None;
        self.album_token = None;
        self.album_neighborhood_id = None;
        self.album_has_image.clear();
    }

    pub(super) fn build_album_snapshot(
        &self,
        nh_id: &str,
        family_id: u16,
        selected_hint: Option<PhotoId>,
    ) -> Result<(AlbumSnapshot, BTreeSet<PhotoId>), String> {
        let summaries = self
            .store
            .albums()
            .list_summaries(nh_id, family_id)
            .map_err(|e| e.to_string())?;
        let family_name = self
            .family_names
            .get(&family_id)
            .cloned()
            .unwrap_or_else(|| format!("Family {family_id}"));
        let focus_idx = selected_hint
            .or_else(|| self.album_dialog.selected_id())
            .and_then(|sel| summaries.iter().position(|s| s.id == sel.0))
            .unwrap_or(0);
        let (win_start, win_end) = album_hydration_window(summaries.len(), focus_idx);
        let mut entries = Vec::with_capacity(summaries.len());
        let mut has_image = BTreeSet::new();
        for (idx, s) in summaries.into_iter().enumerate() {
            let pid = PhotoId(s.id);
            let entry_has_img = s.image_dimensions.is_some();
            if entry_has_img {
                has_image.insert(pid);
            }
            let in_sel = idx >= win_start && idx < win_end;
            let image = if entry_has_img && in_sel {
                self.store
                    .albums()
                    .get(nh_id, family_id, s.id)
                    .map_err(|e| e.to_string())?
                    .and_then(|p| p.image)
                    .map(|img| Image {
                        width: img.width,
                        height: img.height,
                        rgba: img.to_rgba(),
                    })
            } else {
                None
            };
            entries.push(AlbumEntry {
                id: pid,
                title: s.title,
                caption: s.caption,
                image,
                sim_day: s.sim_day,
            });
        }
        Ok((
            AlbumSnapshot {
                family_name,
                entries,
            },
            has_image,
        ))
    }

    /// Loads the shown photo and its neighbors, and drops the rest, so
    /// a large album stays bounded in memory.
    pub(super) fn hydrate_album_images(&mut self) {
        let (Some(nh_id), Some(token)) = (self.album_neighborhood_id.clone(), self.album_token)
        else {
            return;
        };
        let selected = self.album_dialog.selected_id();
        let Some(snapshot) = self.album_snapshot.as_mut() else {
            return;
        };
        let total = snapshot.entries.len();
        if total == 0 {
            return;
        }
        let focus_idx = selected
            .and_then(|sel| snapshot.entries.iter().position(|e| e.id == sel))
            .unwrap_or(0);
        let (win_start, win_end) = album_hydration_window(total, focus_idx);
        for (idx, entry) in snapshot.entries.iter_mut().enumerate() {
            if !(win_start..win_end).contains(&idx) {
                entry.image = None;
            } else if entry.image.is_none()
                && self.album_has_image.contains(&entry.id)
                && let Ok(Some(photo)) =
                    self.store.albums().get(&nh_id, token.family_id, entry.id.0)
                && let Some(img) = photo.image
            {
                entry.image = Some(Image {
                    width: img.width,
                    height: img.height,
                    rgba: img.to_rgba(),
                });
            }
        }
    }

    /// After a delete, native keeps the current index, clamped to the new
    /// last photo (`0x4ee087`): with A B C D, deleting C shows D and
    /// deleting D shows C. Deleting the only photo closes the album
    /// (`0x52b0c8`).
    pub(super) fn after_album_delete(&mut self, index: usize) {
        let Some(snapshot) = self.album_snapshot.clone() else {
            return;
        };
        if snapshot.entries.is_empty() {
            self.close_photo_album();
            return;
        }
        self.album_dialog.show_index(&snapshot, index);
        self.hydrate_album_images();
    }

    pub(super) fn reload_album_snapshot(
        &mut self,
        select_id: Option<PhotoId>,
    ) -> Result<(), String> {
        let (Some(nh_id), Some(token)) = (self.album_neighborhood_id.clone(), self.album_token)
        else {
            return Err("Photo Album is not open".into());
        };
        let (snapshot, has_image) =
            self.build_album_snapshot(&nh_id, token.family_id, select_id)?;
        self.album_has_image = has_image;
        self.album_dialog.sync_snapshot(&snapshot);
        if let Some(id) = select_id {
            self.album_dialog.select_entry(&snapshot, id);
        }
        self.album_snapshot = Some(snapshot);
        self.hydrate_album_images();
        Ok(())
    }

    /// Rebuilds the open album from the store without moving its
    /// selection: the selected photo and any caption being typed stay as
    /// they are, and new photos join the list.
    pub(super) fn refresh_album_snapshot(&mut self) -> Result<(), String> {
        let (Some(nh_id), Some(token)) = (self.album_neighborhood_id.clone(), self.album_token)
        else {
            return Err("Photo Album is not open".into());
        };
        let keep = self.album_dialog.selected_id();
        let (snapshot, has_image) = self.build_album_snapshot(&nh_id, token.family_id, keep)?;
        self.album_has_image = has_image;
        self.album_dialog.sync_snapshot(&snapshot);
        self.album_snapshot = Some(snapshot);
        self.hydrate_album_images();
        Ok(())
    }

    pub(super) fn send_album_key(&mut self, key: AlbumKey) -> ShellOutcome {
        // A notice over the album takes the keys (see `album_notice_open`).
        if self.dialog.is_some() {
            return ShellOutcome::default();
        }
        let cmds = self
            .album_snapshot
            .clone()
            .map(|snap| {
                self.album_dialog
                    .handle_key(key, self.size, &snap, &self.album_art)
            })
            .unwrap_or_default();
        let out = self.execute_album_commands(cmds);
        self.hydrate_album_images();
        out
    }

    pub(super) fn execute_album_commands(&mut self, cmds: Vec<AlbumCommand>) -> ShellOutcome {
        let mut out = ShellOutcome::redraw();
        for cmd in cmds {
            let step = self.execute_album_command(cmd);
            out.redraw |= step.redraw;
            out.quit |= step.quit;
            out.photo |= step.photo;
        }
        out
    }

    /// Executes a single [`AlbumCommand`] emitted by [`PhotoAlbumDialog`] against
    /// REDB's fenced [`sims_sim::album_store::PhotoAlbumStore`].
    pub fn execute_album_command(&mut self, cmd: AlbumCommand) -> ShellOutcome {
        match cmd {
            AlbumCommand::Close => {
                self.close_photo_album();
                ShellOutcome::redraw()
            }
            AlbumCommand::SelectEntry(id) => {
                if let Some(snap) = &self.album_snapshot {
                    self.album_dialog.select_entry(snap, id);
                }
                self.hydrate_album_images();
                ShellOutcome::redraw()
            }
            AlbumCommand::UpdateEntryText { id, title, caption } => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    match self
                        .store
                        .albums()
                        .update_text(&nh_id, token, id.0, &title, &caption)
                    {
                        Ok(_) => {
                            if let Err(err) = self.reload_album_snapshot(Some(id)) {
                                self.set_note(format!("Updated entry; album reload error: {err}"));
                            }
                        }
                        Err(err) => {
                            self.set_note(format!("Could not update album text: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
            AlbumCommand::MoveEntry { id, new_index } => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    match self
                        .store
                        .albums()
                        .move_entry(&nh_id, token, id.0, new_index)
                    {
                        Ok(_) => {
                            if let Err(err) = self.reload_album_snapshot(Some(id)) {
                                self.set_note(format!(
                                    "Reordered entry; album reload error: {err}"
                                ));
                            }
                        }
                        Err(err) => {
                            self.set_note(format!("Could not reorder album entry: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
            AlbumCommand::DeleteEntry(id) => {
                if let (Some(nh_id), Some(token)) =
                    (self.album_neighborhood_id.clone(), self.album_token)
                {
                    let index = self
                        .album_snapshot
                        .as_ref()
                        .and_then(|snap| snap.entries.iter().position(|e| e.id == id))
                        .unwrap_or(0);
                    match self.store.albums().delete(&nh_id, token, id.0) {
                        Ok(_) => match self.reload_album_snapshot(None) {
                            Ok(()) => self.after_album_delete(index),
                            Err(err) => {
                                self.set_note(format!("Deleted entry; album reload error: {err}"));
                            }
                        },
                        Err(err) => {
                            self.set_note(format!("Could not delete album entry: {err}"));
                        }
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }
}
