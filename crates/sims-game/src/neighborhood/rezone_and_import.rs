//! Rezoning, family import, credits and CAF voice previews.

use super::*;

impl NeighborhoodShell {
    // =========================================================================
    // Neighborhood Top-Bar Secondary Actions: Rezone, Import Family, Credits
    // =========================================================================

    pub(super) fn is_lot_rezonable(&self, lot_id: u16) -> bool {
        destination_for_kind(self.kind).is_none()
            && ((1..=10).contains(&lot_id) || (50..=80).contains(&lot_id))
    }

    pub(super) fn zoning_label(&self, zone: i32) -> &str {
        if zone == 2 {
            DialogStrings::get(&self.dialogs.zoning_types, 1, "Community")
        } else {
            DialogStrings::get(&self.dialogs.zoning_types, 0, "Residential")
        }
    }

    pub(super) fn active_trip_blocking_rezone(
        &self,
        stored: &StoredNeighborhood,
        house: u32,
        saved_lot: Option<&[u8]>,
    ) -> Option<String> {
        let house_i32 = i32::try_from(house).unwrap_or(-1);
        for fam in stored.families.values() {
            let sec = fam.secondary_house();
            if fam.house == house_i32 && sec != 0 {
                return Some(format!(
                    "Lot {house} has an active travel trip to lot {sec}; resume or abort the trip before rezoning."
                ));
            }
            if sec == house_i32 && sec != 0 {
                return Some(format!(
                    "Lot {house} is the destination of an active travel trip from lot {}; resume or abort the trip before rezoning.",
                    fam.house
                ));
            }
        }
        if saved_lot.is_some_and(|bytes| bytes.len() >= 4 && &bytes[0..4] == b"TRVL") {
            return Some(format!(
                "Lot {house} has an active travel trip in progress; resume or abort the trip before rezoning."
            ));
        }
        None
    }

    pub(super) fn prompt_rezone_lot(&mut self, lot_id: u16) -> ShellOutcome {
        let title = DialogStrings::get(&self.dialogs.rezone, 0, "Rezone").to_owned();
        if !self.is_lot_rezonable(lot_id) {
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(
                    title,
                    format!("Lot {lot_id} cannot be rezoned."),
                    self.dialogs.ok(),
                )
                .with_frame(FrameStyle::Generic),
            );
            return ShellOutcome::redraw();
        }
        let _ = self.refresh_from_store();
        let house = u32::from(lot_id);
        let saved_lot = self
            .store
            .load_saved_lot(&self.neighborhood_id, house)
            .ok()
            .flatten();
        if let Some(reason) =
            self.active_trip_blocking_rezone(&self.stored, house, saved_lot.as_deref())
        {
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(title, reason, self.dialogs.ok()).with_frame(FrameStyle::Generic),
            );
            return ShellOutcome::redraw();
        }
        let current_zone = self
            .stored
            .zoning
            .get(&(lot_id as i16))
            .copied()
            .unwrap_or(1);
        let target_zone = if current_zone == 2 { 1 } else { 2 };
        let current_label = self.zoning_label(current_zone).to_owned();
        let target_label = self.zoning_label(target_zone).to_owned();
        let base_tmpl = DialogStrings::get(&self.dialogs.rezone, 1, "Rezone %s to %s?");
        let mut body = format_template(base_tmpl, &[&current_label, &target_label]);
        let expected_occupant = self
            .stored
            .families
            .iter()
            .find(|(_, f)| f.house == i32::from(lot_id))
            .map(|(&id, _)| id);
        if let Some(fam_id) = expected_occupant {
            let fam_name = self
                .family_names
                .get(&fam_id)
                .cloned()
                .unwrap_or_else(|| format!("Family {fam_id}"));
            body.push_str(&format!(
                "\n\n{fam_name} will be moved back to the Family Bin before rezoning."
            ));
        }
        self.open_dialog(
            ShellDialogKind::ConfirmRezone {
                lot_id,
                target_zone,
                expected_instance_id: self.stored.instance_id,
                expected_revision: self.stored.revision,
                expected_occupant,
            },
            Message::confirm(title, body, vec![self.dialogs.yes(), self.dialogs.no()])
                .with_frame(FrameStyle::Generic),
        );
        ShellOutcome::redraw()
    }

    /// Toggles `lot_id` zoning in REDB (`1` = Residential, `2` = Community),
    /// atomically staging any household eviction and zoning update together
    /// under strict `(instance_id, revision)` fencing, and refreshes map
    /// metadata and hover overlays immediately.
    pub fn execute_rezone(&mut self, lot_id: u16, target_zone: i32) -> Result<i32, String> {
        self.execute_rezone_checked(lot_id, target_zone, None)
    }

    pub(super) fn execute_rezone_checked(
        &mut self,
        lot_id: u16,
        target_zone: i32,
        expected: Option<(u128, u64, Option<u16>)>,
    ) -> Result<i32, String> {
        if !self.is_lot_rezonable(lot_id) {
            return Err(format!("lot {lot_id} is not eligible for rezoning"));
        }
        if target_zone != 1 && target_zone != 2 {
            return Err(format!("unsupported zoning code {target_zone}"));
        }
        let house = u32::from(lot_id);
        let snapshot = self
            .store
            .load_lot_snapshot(&self.neighborhood_id, house)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("neighborhood {:?} not found", self.neighborhood_id))?;

        if let Some(reason) = self.active_trip_blocking_rezone(
            &snapshot.neighborhood,
            house,
            snapshot.saved_lot.as_deref(),
        ) {
            return Err(reason);
        }

        let live_occupant = snapshot
            .neighborhood
            .families
            .iter()
            .find(|(_, f)| f.house == i32::from(lot_id))
            .map(|(&id, _)| id);

        let (fence_instance_id, fence_revision) =
            if let Some((exp_inst, exp_rev, exp_occ)) = expected {
                if snapshot.neighborhood.instance_id != exp_inst
                    || snapshot.neighborhood.revision != exp_rev
                    || live_occupant != exp_occ
                {
                    return Err(
                    "neighborhood state or lot occupancy changed since confirmation; please retry"
                        .into(),
                );
                }
                (exp_inst, exp_rev)
            } else {
                (
                    snapshot.neighborhood.instance_id,
                    snapshot.neighborhood.revision,
                )
            };

        if live_occupant.is_some() {
            // Stage the eviction in an ephemeral NeighborhoodStore so the
            // evicted lot blob and updated zoning commit together in one
            // fenced REDB transaction (never leaving a partial eviction behind).
            let staging_dir = TempStagingDir::new("rezone-evict")?;
            let staging_store = NeighborhoodStore::open(&staging_dir.path().join("stage.redb"))
                .map_err(|e| e.to_string())?;
            for (file_name, bytes) in &snapshot.character_files {
                staging_store
                    .save_character_file(&self.neighborhood_id, file_name, bytes)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(base_house) = &snapshot.base_house {
                staging_store
                    .save_base_house(&self.neighborhood_id, house, base_house)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(saved_lot) = &snapshot.saved_lot {
                staging_store
                    .save_lot_and_neighborhood(
                        &self.neighborhood_id,
                        house,
                        saved_lot,
                        &snapshot.neighborhood,
                    )
                    .map_err(|e| e.to_string())?;
            } else {
                staging_store
                    .save_neighborhood(&self.neighborhood_id, &snapshot.neighborhood)
                    .map_err(|e| e.to_string())?;
            }
            lifecycle::evict(
                &self.install_root,
                &staging_store,
                &self.neighborhood_id,
                house,
            )
            .map_err(|e| e.to_string())?;
            let evicted_lot_bytes = staging_store
                .load_saved_lot(&self.neighborhood_id, house)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "staged eviction lot missing after evict".to_owned())?;
            let mut evicted_nh = staging_store
                .load_neighborhood(&self.neighborhood_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "staged neighborhood missing after evict".to_owned())?;
            evicted_nh.zoning.insert(lot_id as i16, target_zone);
            self.store
                .save_lot_and_neighborhood_checked(
                    &self.neighborhood_id,
                    house,
                    Some((fence_instance_id, fence_revision)),
                    &evicted_lot_bytes,
                    &evicted_nh,
                    &[],
                )
                .map_err(|e| e.to_string())?
                .map_err(|conflict| format!("neighborhood modified concurrently ({conflict:?})"))?;
        } else {
            let mut updated = snapshot.neighborhood;
            updated.zoning.insert(lot_id as i16, target_zone);
            updated.revision = updated.revision.saturating_add(1);
            self.store
                .save_neighborhood_checked(
                    &self.neighborhood_id,
                    fence_instance_id,
                    fence_revision,
                    &updated,
                )
                .map_err(|e| e.to_string())?
                .map_err(|conflict| format!("neighborhood modified concurrently ({conflict:?})"))?;
        }

        self.refresh_from_store()?;
        self.note = Some(format!(
            "Rezoned lot {lot_id} to {}.",
            self.zoning_label(target_zone)
        ));
        Ok(target_zone)
    }

    /// Adds an extra directory scanned for `.FAM` files by `ImportFamily`
    /// (prepended ahead of the install's `UserData*/Import` and `Export` folders).
    pub fn add_import_search_dir(&mut self, dir: PathBuf) {
        if !self.import_search_dirs.contains(&dir) {
            self.import_search_dirs.insert(0, dir);
        }
        self.refresh_importable_families();
    }

    pub(super) fn scan_importable_families(&self) -> Vec<ImportableFamily> {
        let mut dirs: Vec<(PathBuf, String)> = Vec::new();
        for extra in &self.import_search_dirs {
            let label = extra
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Import")
                .to_owned();
            dirs.push((extra.clone(), label));
        }
        // The shell's own exports ([`Self::family_export_dir`]) can be
        // imported again, as the original's `Export` folder's can.
        let exports = self.family_export_dir();
        if exports.is_dir() && !dirs.iter().any(|(d, _)| d == &exports) {
            dirs.push((exports, "Exported".to_owned()));
        }
        if let Ok(user_dir) = resolve_user_dir(&self.install, self.slot) {
            if let Some(imp) = resolve_dir_case_insensitive(&user_dir, "Import") {
                dirs.push((imp, format!("{}/Import", self.neighborhood_id)));
            }
            if let Some(exp) = resolve_dir_case_insensitive(&user_dir, "Export") {
                dirs.push((exp, format!("{}/Export", self.neighborhood_id)));
            }
        }
        if let Ok(ud1) = self.install.file("UserData") {
            if let Some(imp) = resolve_dir_case_insensitive(&ud1, "Import")
                && !dirs.iter().any(|(d, _)| d == &imp)
            {
                dirs.push((imp, "UserData/Import".into()));
            }
            if let Some(exp) = resolve_dir_case_insensitive(&ud1, "Export")
                && !dirs.iter().any(|(d, _)| d == &exp)
            {
                dirs.push((exp, "UserData/Export".into()));
            }
        }

        let mut seen_paths = BTreeSet::new();
        let mut out = Vec::new();
        for (dir, source_label) in dirs {
            let Ok(read_dir) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<PathBuf> = read_dir
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("fam"))
                })
                .collect();
            entries.sort();
            for path in entries {
                let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if !seen_paths.insert(canon) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(parsed) = FamilyFile::parse(&bytes) else {
                    continue;
                };
                // Skip system NPC/stray households (`Strays_4000.FAM`).
                // Ids past i16::MAX are refused by the importer itself.
                let id = parsed.export_info.family_id;
                if sims_vm::caf::is_reserved_pool_family(id) || id > i16::MAX as u16 {
                    continue;
                }
                let members: Vec<u16> = parsed
                    .export_info
                    .member_ids
                    .into_iter()
                    .take_while(|&id| id != 0)
                    .collect();
                // The importer's member checks (`import_family_fam`): one
                // to eight, ids up to i16::MAX, none twice.
                let mut seen = BTreeSet::new();
                if members.is_empty()
                    || members.len() > sims_vm::caf::MAX_HOUSEHOLD_MEMBERS
                    || !members
                        .iter()
                        .all(|&id| id <= i16::MAX as u16 && seen.insert(id))
                {
                    continue;
                }
                let member_count = members.len();
                let family_name = parsed
                    .iff
                    .get(b"CTSS", 1000)
                    .and_then(|c| StringTable::parse(parsed.iff.data(c)).ok())
                    .and_then(|t| {
                        t.get(0, US_ENGLISH)
                            .or_else(|| t.get(0, 1))
                            .map(str::to_owned)
                    })
                    .or_else(|| {
                        parsed
                            .name
                            .get(0, US_ENGLISH)
                            .or_else(|| parsed.name.get(0, 1))
                            .map(str::to_owned)
                    })
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Family")
                            .to_owned()
                    });
                out.push(ImportableFamily {
                    path,
                    family_name,
                    member_count,
                    budget: parsed.family.budget,
                    source_label: source_label.clone(),
                });
            }
        }
        out
    }

    /// The `.FAM` files the Import picker would offer, in the order it
    /// lists them.
    pub fn importable_family_paths(&self) -> Vec<PathBuf> {
        self.scan_importable_families()
            .into_iter()
            .map(|f| f.path)
            .collect()
    }

    /// Rescans configured `Import/` and `Export/` folders for `.FAM` archives
    /// and updates the top-bar Import button enabled state. Returns `true` if
    /// availability changed.
    pub fn refresh_importable_families(&mut self) -> bool {
        let next = !self.scan_importable_families().is_empty();
        if next != self.nbhd_state.has_importable_family {
            self.nbhd_state.has_importable_family = next;
            true
        } else {
            false
        }
    }

    /// Scans installed `Import/` and `Export/` `.FAM` archives and opens the
    /// native Family Import picker modal.
    pub fn open_import_family_picker(&mut self) -> bool {
        let candidates = self.scan_importable_families();
        self.nbhd_state.has_importable_family = !candidates.is_empty();
        let title = DialogStrings::get(&self.dialogs.import_strs, 8, "Import Family").to_owned();
        if candidates.is_empty() {
            self.open_dialog(
                ShellDialogKind::Notice,
                Message::notice(
                    title,
                    "No importable .FAM family files were found in UserData/Import.",
                    self.dialogs.ok(),
                )
                .with_frame(FrameStyle::Generic),
            );
            return false;
        }
        let mut items = Vec::with_capacity(candidates.len());
        let mut entries = BTreeMap::new();
        for (idx, fam) in candidates.into_iter().enumerate() {
            let id = format!("import_fam:{idx}");
            let file_name = fam
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("family.FAM");
            let members_word = if fam.member_count == 1 { "Sim" } else { "Sims" };
            let description = format!(
                "{} {} | {} | {}/{}",
                fam.member_count,
                members_word,
                format_simoleons(fam.budget),
                fam.source_label,
                file_name,
            );
            items.push(ShopItem {
                id: id.clone(),
                name: fam.family_name.clone(),
                description,
                price: 0,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: false,
                owned: false,
                available: true,
            });
            entries.insert(id, fam);
        }
        self.clear_hover_state();
        self.import_picker_entries = entries;
        self.import_picker_snapshot = Some(ShopSnapshot {
            kind: ShopKind::DowntownBoutique(WardrobeCategory::Everyday),
            title_override: Some(title),
            buy_label_override: Some("Import".into()),
            shopper_name: None,
            wallet: Wallet::default(),
            items,
            tried_on_item_id: None,
            preview_image: None,
            preview_label: None,
            hide_wallet_and_prices: true,
            choice: None,
            body_shapes: Vec::new(),
        });
        self.import_picker.open();
        true
    }

    pub fn close_import_family_picker(&mut self) {
        self.import_picker.close();
        self.import_picker_snapshot = None;
        self.import_picker_entries.clear();
    }

    pub(super) fn handle_import_picker_command(&mut self, cmd: ShopCommand) -> ShellOutcome {
        match cmd {
            ShopCommand::Cancel => {
                self.close_import_family_picker();
                ShellOutcome::redraw()
            }
            // The import picker is a list, never a choice dialog.
            ShopCommand::TryOn { .. }
            | ShopCommand::RotatePreview(_)
            | ShopCommand::NoSelection => ShellOutcome::redraw(),
            ShopCommand::Buy { item_id, .. } => {
                let Some(entry) = self.import_picker_entries.get(&item_id).cloned() else {
                    return ShellOutcome::redraw();
                };
                self.close_import_family_picker();
                let title =
                    DialogStrings::get(&self.dialogs.import_strs, 8, "Import Family").to_owned();
                match self.execute_import_family(&entry) {
                    Ok(created) => {
                        let body = format!(
                            "{} imported into the Family Bin ({} members).",
                            created.family_name,
                            created.members.len()
                        );
                        self.note = Some(body.clone());
                        self.open_dialog(
                            ShellDialogKind::Notice,
                            Message::notice(title, body, self.dialogs.ok())
                                .with_frame(FrameStyle::Generic),
                        );
                    }
                    Err(e) => {
                        let body = format!("Could not import {}: {e}", entry.family_name);
                        self.note = Some(body.clone());
                        self.open_dialog(
                            ShellDialogKind::Notice,
                            Message::notice(title, body, self.dialogs.ok())
                                .with_frame(FrameStyle::Generic),
                        );
                    }
                }
                ShellOutcome::redraw()
            }
        }
    }

    /// Where the Family Bin's Export button writes `.FAM` files:
    /// `<saves file name>.userdata/<neighborhood id>/Export/`, beside the
    /// saves file and never in the install, as the shell's
    /// `Neighborhood.iff` is ([`Self::neighborhood_iff_path`]). The
    /// original writes its exports to its `UserData*/Export` folder.
    pub fn family_export_dir(&self) -> PathBuf {
        self.neighborhood_iff_dir().join("Export")
    }

    /// Exports family `family_id` from the Family Bin as a `.FAM` file in
    /// [`Self::family_export_dir`], with the family's `FAMh` if its save
    /// inputs hold one ([`sims_vm::caf::export_stored_family_fam`], the
    /// routine 0x428a17 calls, with `FINV` as the original's exports have).
    /// The file is named `<family name>_<family id>.FAM`, as the original
    /// names its exports, and replaces an earlier export of the same
    /// family. It is written to a temporary file first and renamed, so a
    /// failed write never leaves half a file under the final name.
    /// Returns the file written.
    pub fn export_family(&mut self, family_id: u16) -> Result<PathBuf, String> {
        if !self.stored.families.contains_key(&family_id) {
            return Err(format!("family {family_id} is not in this neighborhood"));
        }
        let name = self
            .family_names
            .get(&family_id)
            .cloned()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Family {family_id}"));
        let bytes = sims_vm::caf::export_stored_family_fam(
            &self.store,
            &self.neighborhood_id,
            family_id,
            &name,
            US_ENGLISH,
            true,
        )
        .map_err(|e| e.to_string())?;
        let dir = self.family_export_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(export_file_name(&name, family_id));
        write_replacing(&path, &bytes)?;
        self.refresh_importable_families();
        Ok(path)
    }

    /// Exports family `family_id` ([`Self::export_family`]) and tells the
    /// player where the file went, or why it could not be written.
    pub(super) fn export_family_with_notice(&mut self, family_id: u16) {
        let name = self
            .family_names
            .get(&family_id)
            .cloned()
            .unwrap_or_else(|| format!("Family {family_id}"));
        let title = "Export Family".to_owned();
        let body = match self.export_family(family_id) {
            Ok(path) => format!("{name} was exported to {}.", path.display()),
            Err(e) => format!("Could not export {name}: {e}"),
        };
        self.note = Some(body.clone());
        self.open_dialog(
            ShellDialogKind::Notice,
            Message::notice(title, body, self.dialogs.ok()).with_frame(FrameStyle::Generic),
        );
    }

    pub(super) fn execute_import_family(
        &mut self,
        entry: &ImportableFamily,
    ) -> Result<sims_vm::caf::CreatedHousehold, String> {
        let raw_bytes = std::fs::read(&entry.path).map_err(|e| e.to_string())?;
        let parsed = FamilyFile::parse(&raw_bytes).map_err(|e| e.to_string())?;
        let fam_bytes = if parsed.family.house == 0 && parsed.family.secondary_house() == 0 {
            raw_bytes
        } else {
            // Normalize a house-exported `.FAM` into a Family Bin archive (`house = 0`,
            // `secondary_house = 0`) so `import_family_fam` places it cleanly in the Family Bin.
            let mut bin_family = parsed.family.clone();
            bin_family.house = 0;
            if bin_family.version >= 8 && bin_family.tail.len() >= 4 {
                bin_family.tail[0..4].fill(0);
            }
            let fami_id = parsed.export_info.family_id;
            let fami_bytes = encode_fami(&bin_family);
            // Rewrite ONLY the IFF's effective resources in `resource_order`
            // (preserving native `rsmp` lookup semantics and omitting stale,
            // unlisted physical chunks so reparse never exposes shadowed data).
            let tags: BTreeSet<[u8; 4]> = parsed
                .iff
                .chunks
                .iter()
                .map(|c| c.tag)
                .filter(|tag| tag != b"rsmp" && tag != b"XXXX")
                .collect();
            let mut chunks: Vec<ChunkData> = Vec::with_capacity(parsed.iff.chunks.len());
            for tag in tags {
                let effective: Vec<&sims_formats::iff::Chunk> =
                    match parsed.iff.resource_order(&tag) {
                        Ok(list) => list,
                        Err(_) => parsed.iff.all(&tag).collect(),
                    };
                for chunk in effective {
                    let eff_flags = parsed.iff.resource_flags(chunk);
                    // Clear `DELETED` (`4`) when emitting a mapless IFF so an
                    // `rsmp`-listed live resource is never skipped by the
                    // physical chunk scanner on reparse.
                    let write_flags = eff_flags & !sims_formats::iff::DELETED;
                    let data =
                        if &chunk.tag == b"FAMI" && chunk.id == fami_id && (eff_flags >> 8) == 0 {
                            fami_bytes.clone()
                        } else {
                            parsed.iff.data(chunk).to_vec()
                        };
                    chunks.push(ChunkData {
                        tag: chunk.tag,
                        id: chunk.id,
                        flags: write_flags,
                        label: chunk.label.clone(),
                        data,
                    });
                }
            }
            write_iff(&chunks).map_err(|e| e.to_string())?
        };

        if self.import_caf_ctx.is_none() {
            self.import_caf_ctx =
                Some(CafContext::from_install(&self.install_root).map_err(|e| e.to_string())?);
        }
        let ctx = self.import_caf_ctx.as_ref().expect("import_caf_ctx");
        let seed = self
            .import_rng_seed
            .wrapping_add(self.clock_us as u32)
            .max(1);
        self.import_rng_seed = self.import_rng_seed.wrapping_add(1).max(1);
        let mut rng = CrtRng::from_state(seed);
        let created = match import_family_fam(
            ctx,
            &self.store,
            &self.neighborhood_id,
            &fam_bytes,
            US_ENGLISH,
            &mut rng,
        ) {
            Ok(created) => created,
            Err(sims_vm::caf::CafError::Content(sims_content::Error::Unsupported(_)))
                if ctx.wardrobe.is_some() =>
            {
                // Some base-game / pre-Hot-Date `.FAM` archives hit unported
                // legacy outfit replacement generators in `refresh_outfits`;
                // fall back to preserving the archive's original body strings.
                let fallback_ctx = CafContext {
                    template_iff: ctx.template_iff.clone(),
                    catalog_guids: ctx.catalog_guids.clone(),
                    wardrobe: None,
                    starting_funds: ctx.starting_funds,
                };
                let mut fallback_rng = CrtRng::from_state(seed);
                import_family_fam(
                    &fallback_ctx,
                    &self.store,
                    &self.neighborhood_id,
                    &fam_bytes,
                    US_ENGLISH,
                    &mut fallback_rng,
                )
                .map_err(|e| e.to_string())?
            }
            Err(err) => return Err(err.to_string()),
        };
        self.family_names
            .insert(created.family_id, created.family_name.clone());
        self.refresh_from_store()?;
        Ok(created)
    }

    /// Opens the native paginated Credits/About modal loaded from
    /// `GameData/Credits.iff` (with fallback to `GameData/UIText.iff` `STR# 163`).
    pub fn open_credits_modal(&mut self) {
        let title = self
            .chrome_strings
            .bar
            .get(sims_ui::neighborhood::BarButton::Credits.tooltip_index())
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "Credits".into());
        let mut pages =
            sims_ui::book::load_credits_pages(&self.install, US_ENGLISH).unwrap_or_default();
        if pages.is_empty() {
            pages.push(BookPage {
                heading: None,
                category: None,
                entries: vec![BookEntry {
                    title: title.clone(),
                    subtitle: None,
                    body: "Credits data is unavailable in this install.".into(),
                    badges: Vec::new(),
                    highlight: false,
                }],
            });
        }
        self.clear_hover_state();
        self.credits_snapshot = Some(ReferenceBookSnapshot {
            title,
            subtitle: None,
            held_summary: Vec::new(),
            pages,
            use_magic_book_chrome: false,
        });
        self.credits_dialog.open();
    }

    /// Closes the credits book and, when it was open over a map, restores
    /// the map's music, as every native close path does (`station_dtnhood`
    /// `0x54b879`, `station_superstar` `0x565bf8`, `station_vacation`
    /// `0x56a5ad`, `music_magictown` `0x552fc5`, `bkground_nhood1`
    /// `0x5593ff`).
    pub fn close_credits_modal(&mut self) {
        if !self.credits_dialog.is_open() {
            self.drop_credits_modal();
            return;
        }
        self.drop_credits_modal();
        if matches!(self.screen, ActiveScreen::Neighborhood) {
            self.audio_queue
                .push(Input::MusicMode(music_mode_for_kind(self.kind)));
        }
    }

    /// Discards the credits book without touching the music, for resets
    /// whose caller sets the music itself (entering a map, switching maps).
    pub(super) fn drop_credits_modal(&mut self) {
        self.credits_dialog.close();
        self.credits_snapshot = None;
    }

    // =========================================================================
    // CAF Simlish Voice Preview draining
    // =========================================================================

    pub(super) fn drain_caf_voice_previews(&mut self) {
        let previews = match &mut self.screen {
            ActiveScreen::CreateFamily(caf) => caf.take_voice_previews(),
            _ => Vec::new(),
        };
        for preview in previews {
            self.audio_queue.push(Input::PlayVoice {
                voice: preview.voice,
                category: preview.category,
                pitch: preview.pitch,
                seed: preview.seed,
                object: None,
            });
        }
    }
}

/// Writes `bytes` to `path`, replacing it whole or not at all: they go to a
/// new file beside it, made for this write alone (never one already there,
/// nor a link), which is closed and then renamed over `path`. The new
/// file is removed if any step fails.
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let shown = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let (partial, mut file) = loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Short, whatever the final name's length.
        let partial = path.with_file_name(format!(".export.{}.{n}.partial", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
        {
            Ok(file) => break (partial, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(shown(&partial, e)),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written
        .map_err(|e| shown(&partial, e))
        .and_then(|()| std::fs::rename(&partial, path).map_err(|e| shown(path, e)));
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// The file name the original gives a family export, `<family name>_<id>.FAM`
/// (`Export/<name>_<id>.FAM`). Characters a file name cannot hold on every
/// platform the game runs on become `_`; an empty name becomes `Family`.
pub(super) fn export_file_name(name: &str, family_id: u16) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(['.', ' ']);
    let stem = if cleaned.is_empty() {
        "Family"
    } else {
        cleaned
    };
    // Windows takes a name whose part before the first dot is a device
    // (`NUL.backup`, `COM¹`) as that device: such a name gets a leading `_`.
    let device: Vec<char> = stem
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let reserved = match device.as_slice() {
        ['C', 'O', 'N'] | ['P', 'R', 'N'] | ['A', 'U', 'X'] | ['N', 'U', 'L'] => true,
        ['C', 'O', 'M', n] | ['L', 'P', 'T', n] => matches!(n, '1'..='9' | '¹' | '²' | '³'),
        _ => false,
    };
    let prefix = if reserved { "_" } else { "" };
    let tail = format!("_{family_id}.FAM");
    // A file name is at most 255 bytes on the file systems the game runs
    // on: a longer family name is cut, at a character, to fit. The family
    // keeps its whole name inside the file.
    let room = FILE_NAME_MOST - prefix.len() - tail.len();
    let mut cut = stem.len().min(room);
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    let stem = stem[..cut].trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() { "Family" } else { stem };
    format!("{prefix}{stem}{tail}")
}

/// The longest file name, in bytes, the export writes.
const FILE_NAME_MOST: usize = 255;

#[cfg(test)]
mod export_name_tests {
    use super::{export_file_name, write_replacing};

    #[test]
    fn an_export_replaces_the_file_whole_and_leaves_nothing_else() {
        let dir =
            std::env::temp_dir().join(format!("sims-game-export-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Newbie_7.FAM");
        write_replacing(&path, b"first").unwrap();
        write_replacing(&path, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second, longer");
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        names.sort();
        assert_eq!(names, ["Newbie_7.FAM"], "no partial file is left");
        // A name as long as a file name can be still writes: the temporary
        // file's name is short.
        let long = dir.join(export_file_name(&"a".repeat(248), 7));
        write_replacing(&long, b"long").unwrap();
        assert_eq!(std::fs::read(&long).unwrap(), b"long");
        std::fs::remove_file(&long).unwrap();
        // A folder that is not there: an error, and nothing made.
        let missing = dir.join("gone").join("x.FAM");
        assert!(write_replacing(&missing, b"x").is_err());
        assert!(!dir.join("gone").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_names_follow_name_underscore_id_and_stay_portable() {
        assert_eq!(export_file_name("Newbie", 7), "Newbie_7.FAM");
        assert_eq!(export_file_name("  Two Words ", 12), "Two Words_12.FAM");
        assert_eq!(export_file_name("a/b\\c:d*e?f", 3), "a_b_c_d_e_f_3.FAM");
        assert_eq!(export_file_name("", 4), "Family_4.FAM");
        assert_eq!(export_file_name("..", 4), "Family_4.FAM");
        assert_eq!(export_file_name("x\u{0}y", 5), "x_y_5.FAM");
        // Device names, alone or before a dot, in any case.
        assert_eq!(export_file_name("NUL.backup", 7), "_NUL.backup_7.FAM");
        assert_eq!(export_file_name("con", 7), "_con_7.FAM");
        assert_eq!(export_file_name("Com1", 7), "_Com1_7.FAM");
        assert_eq!(export_file_name("lpt9.x", 7), "_lpt9.x_7.FAM");
        assert_eq!(export_file_name("COM0", 7), "COM0_7.FAM");
        assert_eq!(export_file_name("Console", 7), "Console_7.FAM");
        assert_eq!(export_file_name("Nullson", 7), "Nullson_7.FAM");
        // A long name still writes: the temporary name is short.
        assert_eq!(export_file_name(&"a".repeat(248), 7).len(), 254);
        // Superscript device digits.
        assert_eq!(
            export_file_name("COM\u{b9}.backup", 7),
            "_COM\u{b9}.backup_7.FAM"
        );
        assert_eq!(export_file_name("lpt\u{b3}", 7), "_lpt\u{b3}_7.FAM");
        // Too long for a file name: cut at a character to 255 bytes.
        for name in [
            "\u{20ac}".repeat(100),
            "a".repeat(300),
            "NUL".to_owned() + &"x".repeat(300),
        ] {
            let file = export_file_name(&name, 32767);
            assert!(file.len() <= 255, "{} bytes", file.len());
            assert!(file.ends_with("_32767.FAM"));
        }
        assert_eq!(
            export_file_name(&"\u{20ac}".repeat(100), 7),
            "\u{20ac}".repeat(83) + "_7.FAM",
            "249 / 3 whole characters"
        );
    }
}
