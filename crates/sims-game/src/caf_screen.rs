//! Interactive Create-a-Family (`CAF`) and Create-a-Sim session bridging
//! [`sims_ui::caf::CafController`], [`sims_people::caf_preview`] (live 3D
//! turntable preview + 6-portrait BMP rasterizer), and [`sims_vm::caf`]
//! (atomic REDB household commit into the Family Bin).

use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_core::CrtRng;
use sims_formats::bitmap::Image;
use sims_people::appearance::Body;
use sims_people::caf_preview::{
    self, CafCatalog, PortraitKind as PreviewPortraitKind, SimConfig, TURNTABLE_BASE_YAW,
    TurntableParams,
};
use sims_people::suit::Wardrobe;
use sims_sim::neighborhood_store::NeighborhoodStore;
use sims_ui::art::Art;
use sims_ui::caf::{
    self as ui_caf, CafCommand, CafController, CafScreen, CafTexts, FamilyDraft, SimAppearance,
    SimDraft, Trait,
};
use sims_ui::frame::Frame;
use sims_vm::caf::{
    self as vm_caf, CafContext, CreatedHousehold, FamilySpec, Personality as VmPersonality,
    PortraitBitmaps, SimSpec,
};
use std::path::{Path, PathBuf};

/// Outcome returned by [`CafSession`] input methods to [`crate::neighborhood::NeighborhoodShell`].
#[derive(Debug, Clone)]
pub enum CafOutcome {
    /// Stay on the Create-a-Family screen; `redraw` indicates visual changes.
    Stay { redraw: bool },
    /// Player cancelled Create-a-Family and should return to the Family Bin.
    Cancelled,
    /// Player confirmed a valid household ready to commit to REDB.
    Commit(FamilyDraft),
}

impl CafOutcome {
    fn redraw() -> Self {
        Self::Stay { redraw: true }
    }
}

/// Self-contained interactive Create-a-Family session.
pub struct CafSession {
    install_root: PathBuf,
    controller: CafController,
    art: Art,
    people_assets: sims_people::Assets,
    wardrobe: Wardrobe,
    catalog_key: Option<(caf_preview::Age, caf_preview::Gender, caf_preview::SkinTone)>,
    catalog: CafCatalog,
    body_key: Option<(
        caf_preview::Age,
        caf_preview::Gender,
        caf_preview::SkinTone,
        usize,
        usize,
    )>,
    active_body: Option<Body>,
    preview_image: Option<Image>,
    clock_us: u64,
    turntable_ms: u64,
    manual_yaw: bool,
    preview_dirty: bool,
    vm_ctx: Option<CafContext>,
    note: Option<String>,
}

impl CafSession {
    /// Loads CAF UI artwork, strings, and 3D Sim wardrobe assets from `install`.
    pub fn open(install_root: &Path, install: &Install, ui: &UiGraphics) -> Result<Self, String> {
        let art = Art::load(ui, US_ENGLISH, &ui_caf::RESOURCES).map_err(|e| e.to_string())?;
        let texts = CafTexts::load(install, US_ENGLISH).unwrap_or_default();
        let people_assets = sims_people::Assets::scan(install_root)?;
        let wardrobe = Wardrobe::of(&people_assets);
        Ok(Self {
            install_root: install_root.to_path_buf(),
            controller: CafController::new(texts),
            art,
            people_assets,
            wardrobe,
            catalog_key: None,
            catalog: CafCatalog::default(),
            body_key: None,
            active_body: None,
            preview_image: None,
            clock_us: 0,
            turntable_ms: 0,
            manual_yaw: false,
            preview_dirty: true,
            vm_ctx: None,
            note: None,
        })
    }

    pub fn controller(&self) -> &CafController {
        &self.controller
    }

    pub fn controller_mut(&mut self) -> &mut CafController {
        &mut self.controller
    }

    pub fn screen(&self) -> CafScreen {
        self.controller.screen()
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub fn set_note(&mut self, note: impl Into<String>) {
        self.note = Some(note.into());
    }

    /// Returns the currently rendered 3D Sim turntable preview image, if on the
    /// Create-a-Sim sub-screen.
    pub fn preview_image(&self) -> Option<&Image> {
        self.preview_image.as_ref()
    }

    /// Advances the caret blink timer and the 3D turntable sweep when editing a
    /// Sim. Returns `true` when the screen should redraw.
    pub fn tick(&mut self, delta_us: u64) -> bool {
        self.clock_us = self.clock_us.saturating_add(delta_us);
        let mut redraw = self.controller.tick(self.clock_us);
        if self.controller.screen() == CafScreen::EditSim && !self.manual_yaw && delta_us > 0 {
            let next_ms = self.clock_us / 1_000;
            if next_ms.saturating_sub(self.turntable_ms) >= 33 {
                self.turntable_ms = next_ms;
                self.preview_dirty = true;
                redraw = true;
            }
        }
        redraw
    }

    pub fn pointer_move(&mut self, point: [i32; 2], frame_size: [u32; 2]) -> CafOutcome {
        let cmd = self.controller.pointer_move(point[0], point[1], frame_size);
        self.handle_command(cmd, true)
    }

    pub fn pointer_down(&mut self, point: [i32; 2], frame_size: [u32; 2]) -> CafOutcome {
        self.note = None;
        let cmd = self.controller.pointer_down(point[0], point[1], frame_size);
        self.handle_command(cmd, true)
    }

    pub fn pointer_up(&mut self) -> CafOutcome {
        self.controller.pointer_up();
        CafOutcome::redraw()
    }

    pub fn key_char(&mut self, ch: char) -> CafOutcome {
        let changed = self.controller.key_char(ch);
        CafOutcome::Stay { redraw: changed }
    }

    pub fn key_backspace(&mut self) -> CafOutcome {
        let changed = self.controller.key_backspace();
        CafOutcome::Stay { redraw: changed }
    }

    pub fn key_delete(&mut self) -> CafOutcome {
        let changed = self.controller.key_delete();
        CafOutcome::Stay { redraw: changed }
    }

    pub fn key_left(&mut self) -> CafOutcome {
        self.controller.key_left();
        CafOutcome::redraw()
    }

    pub fn key_right(&mut self) -> CafOutcome {
        self.controller.key_right();
        CafOutcome::redraw()
    }

    pub fn key_home(&mut self) -> CafOutcome {
        self.controller.key_home();
        CafOutcome::redraw()
    }

    pub fn key_end(&mut self) -> CafOutcome {
        self.controller.key_end();
        CafOutcome::redraw()
    }

    pub fn key_tab(&mut self) -> CafOutcome {
        self.controller.key_tab();
        CafOutcome::redraw()
    }

    pub fn key_enter(&mut self) -> CafOutcome {
        let cmd = self.controller.key_enter();
        self.handle_command(cmd, true)
    }

    pub fn key_escape(&mut self) -> CafOutcome {
        let cmd = self.controller.key_escape();
        self.handle_command(cmd, true)
    }

    /// Synchronizes the 3D turntable preview (if dirty) and draws the active
    /// Create-a-Family or Create-a-Sim screen into `frame`.
    pub fn draw(&mut self, frame: &mut Frame) {
        if self.controller.screen() == CafScreen::EditSim {
            self.sync_active_sim_preview();
        }
        self.controller
            .draw(frame, &self.art, self.preview_image.as_ref());
    }

    /// Converts `draft` into a [`FamilySpec`] (rasterizing all 6 native BMP
    /// portraits per Sim via [`caf_preview::render_portrait`]) and atomically
    /// commits the household into `store`'s REDB Family Bin.
    pub fn commit_family_draft(
        &mut self,
        store: &NeighborhoodStore,
        neighborhood_id: &str,
        draft: &FamilyDraft,
    ) -> Result<CreatedHousehold, String> {
        let mut sim_specs = Vec::with_capacity(draft.members.len());
        for member in &draft.members {
            sim_specs.push(self.build_sim_spec(member)?);
        }
        let spec = FamilySpec::new(draft.last_name.trim(), draft.bio.trim(), sim_specs);
        if self.vm_ctx.is_none() {
            self.vm_ctx =
                Some(CafContext::from_install(&self.install_root).map_err(|e| e.to_string())?);
        }
        let ctx = self.vm_ctx.as_ref().expect("vm_ctx initialized");
        let seed = (self.clock_us as u32)
            .wrapping_add(
                u32::try_from(draft.members.len())
                    .unwrap_or(1)
                    .wrapping_mul(31),
            )
            .max(1);
        let mut rng = CrtRng::from_state(seed);
        vm_caf::commit_family_with_context(ctx, store, neighborhood_id, &spec, &mut rng)
            .map_err(|e| e.to_string())
    }

    fn handle_command(&mut self, cmd: Option<CafCommand>, default_redraw: bool) -> CafOutcome {
        match cmd {
            None => CafOutcome::Stay {
                redraw: default_redraw,
            },
            Some(CafCommand::CancelFamily) => CafOutcome::Cancelled,
            Some(CafCommand::CommitFamily(draft)) => CafOutcome::Commit(draft),
            Some(CafCommand::OpenSimEditor { .. }) => {
                self.manual_yaw = false;
                self.preview_dirty = true;
                self.sync_active_sim_preview();
                CafOutcome::redraw()
            }
            Some(CafCommand::AppearanceChanged(_)) => {
                self.preview_dirty = true;
                self.sync_active_sim_preview();
                CafOutcome::redraw()
            }
            Some(CafCommand::RotatePreview(_)) => {
                self.manual_yaw = true;
                self.preview_dirty = true;
                self.sync_active_sim_preview();
                CafOutcome::redraw()
            }
            Some(CafCommand::SimSaved { index, .. }) => {
                if let Some(member) = self.controller.family().members.get(index).cloned()
                    && let Some(portrait) = self.render_roster_portrait(&member.appearance)
                {
                    self.controller.set_member_portrait(index, Some(portrait));
                }
                self.active_body = None;
                self.body_key = None;
                self.preview_image = None;
                CafOutcome::redraw()
            }
            Some(CafCommand::SimDeleted(_)) | Some(CafCommand::CancelSim) => {
                self.active_body = None;
                self.body_key = None;
                self.preview_image = None;
                CafOutcome::redraw()
            }
        }
    }

    fn ensure_catalog(&mut self, appearance: &SimAppearance) {
        let key = (
            to_preview_age(appearance.age),
            to_preview_gender(appearance.gender),
            to_preview_skin(appearance.skin_tone),
        );
        if self.catalog_key != Some(key) {
            self.catalog =
                caf_preview::enumerate_catalog_from_wardrobe(&self.wardrobe, key.0, key.1, key.2);
            self.catalog_key = Some(key);
        }
        self.controller
            .set_skin_catalog_counts(self.catalog.heads.len(), self.catalog.bodies.len());
    }

    fn sync_active_sim_preview(&mut self) {
        let Some(appearance) = self.controller.editing_sim().map(|s| s.appearance.clone()) else {
            return;
        };
        self.ensure_catalog(&appearance);
        // Re-read clamped indices after `set_skin_catalog_counts`.
        let Some(appearance) = self.controller.editing_sim().map(|s| s.appearance.clone()) else {
            return;
        };
        let age = to_preview_age(appearance.age);
        let gender = to_preview_gender(appearance.gender);
        let skin = to_preview_skin(appearance.skin_tone);
        let bkey = (
            age,
            gender,
            skin,
            appearance.head_index,
            appearance.body_index,
        );
        if self.body_key != Some(bkey) {
            self.body_key = Some(bkey);
            self.active_body = self
                .catalog
                .heads
                .get(appearance.head_index)
                .zip(self.catalog.bodies.get(appearance.body_index))
                .and_then(|(head, body)| {
                    SimConfig::from_entries(age, gender, skin, head, body)
                        .load_body(&self.people_assets)
                        .ok()
                });
            self.preview_dirty = true;
        }
        if !self.preview_dirty {
            return;
        }
        self.preview_dirty = false;
        let Some(body) = &self.active_body else {
            self.preview_image = None;
            return;
        };
        let yaw = if self.manual_yaw {
            TURNTABLE_BASE_YAW + appearance.preview_yaw_deg.to_radians()
        } else {
            caf_preview::turntable_yaw(self.turntable_ms)
        };
        let params = TurntableParams {
            yaw,
            ..TurntableParams::default()
        };
        self.preview_image = caf_preview::render_turntable(body, None, &params)
            .ok()
            .map(canvas_to_image);
    }

    fn render_roster_portrait(&self, appearance: &SimAppearance) -> Option<Image> {
        let age = to_preview_age(appearance.age);
        let gender = to_preview_gender(appearance.gender);
        let skin = to_preview_skin(appearance.skin_tone);
        let catalog =
            caf_preview::enumerate_catalog_from_wardrobe(&self.wardrobe, age, gender, skin);
        let head = catalog
            .heads
            .get(appearance.head_index)
            .or_else(|| catalog.heads.first())?;
        let body_entry = catalog
            .bodies
            .get(appearance.body_index)
            .or_else(|| catalog.bodies.first())?;
        let body = SimConfig::from_entries(age, gender, skin, head, body_entry)
            .load_body(&self.people_assets)
            .ok()?;
        let canvas = caf_preview::render_portrait(&body, PreviewPortraitKind::WebImage2007).ok()?;
        Some(canvas_to_image(canvas))
    }

    fn build_sim_spec(&self, draft: &SimDraft) -> Result<SimSpec, String> {
        let p_age = to_preview_age(draft.appearance.age);
        let p_gender = to_preview_gender(draft.appearance.gender);
        let p_skin = to_preview_skin(draft.appearance.skin_tone);
        let catalog =
            caf_preview::enumerate_catalog_from_wardrobe(&self.wardrobe, p_age, p_gender, p_skin);
        let head = catalog
            .heads
            .get(draft.appearance.head_index)
            .or_else(|| catalog.heads.first())
            .ok_or_else(|| "no CAF head assets found for selected age/gender/skin".to_owned())?;
        let body_entry = catalog
            .bodies
            .get(draft.appearance.body_index)
            .or_else(|| catalog.bodies.first())
            .ok_or_else(|| "no CAF body assets found for selected age/gender/skin".to_owned())?;

        let portraits = SimConfig::from_entries(p_age, p_gender, p_skin, head, body_entry)
            .load_body(&self.people_assets)
            .ok()
            .and_then(|body| render_all_six_portraits(&body).ok());

        let personality = VmPersonality::new(
            draft.personality.get(Trait::Neat),
            draft.personality.get(Trait::Outgoing),
            draft.personality.get(Trait::Active),
            draft.personality.get(Trait::Playful),
            draft.personality.get(Trait::Nice),
        )
        .map_err(|e| e.to_string())?;

        Ok(SimSpec {
            first_name: draft.first_name.trim().to_owned(),
            bio: draft.bio.trim().to_owned(),
            age: to_vm_age(draft.appearance.age),
            gender: to_vm_gender(draft.appearance.gender),
            skin_tone: to_vm_skin(draft.appearance.skin_tone),
            body_appearance: body_entry.appearance.clone(),
            body_texture: body_entry.texture_name.clone(),
            head_appearance: head.appearance.clone(),
            head_texture: head.texture_name.clone(),
            personality,
            portraits,
        })
    }
}

fn render_all_six_portraits(body: &Body) -> Result<PortraitBitmaps, String> {
    let faces = canvas_to_image(caf_preview::render_portrait(
        body,
        PreviewPortraitKind::Faces2002,
    )?);
    let rel = canvas_to_image(caf_preview::render_portrait(
        body,
        PreviewPortraitKind::Relationship2003,
    )?);
    let new_img = canvas_to_image(caf_preview::render_portrait(
        body,
        PreviewPortraitKind::NewImage2004,
    )?);
    let speech_med = canvas_to_image(caf_preview::render_portrait(
        body,
        PreviewPortraitKind::SpeechMed2005,
    )?);
    let speech_lg = canvas_to_image(caf_preview::render_portrait(
        body,
        PreviewPortraitKind::SpeechLarge2006,
    )?);
    let web = canvas_to_image(caf_preview::render_portrait(
        body,
        PreviewPortraitKind::WebImage2007,
    )?);
    PortraitBitmaps::from_images(&faces, &rel, &new_img, &speech_med, &speech_lg, &web)
        .map_err(|e| e.to_string())
}

fn canvas_to_image(canvas: sims_people::raster::Canvas) -> Image {
    Image {
        width: canvas.width,
        height: canvas.height,
        rgba: canvas.rgba,
    }
}

const fn to_preview_age(age: ui_caf::Age) -> caf_preview::Age {
    match age {
        ui_caf::Age::Adult => caf_preview::Age::Adult,
        ui_caf::Age::Child => caf_preview::Age::Child,
    }
}

const fn to_preview_gender(gender: ui_caf::Gender) -> caf_preview::Gender {
    match gender {
        ui_caf::Gender::Male => caf_preview::Gender::Male,
        ui_caf::Gender::Female => caf_preview::Gender::Female,
    }
}

const fn to_preview_skin(skin: ui_caf::SkinTone) -> caf_preview::SkinTone {
    match skin {
        ui_caf::SkinTone::Light => caf_preview::SkinTone::Light,
        ui_caf::SkinTone::Medium => caf_preview::SkinTone::Medium,
        ui_caf::SkinTone::Dark => caf_preview::SkinTone::Dark,
    }
}

const fn to_vm_age(age: ui_caf::Age) -> vm_caf::Age {
    match age {
        ui_caf::Age::Adult => vm_caf::Age::Adult,
        ui_caf::Age::Child => vm_caf::Age::Child,
    }
}

const fn to_vm_gender(gender: ui_caf::Gender) -> vm_caf::Gender {
    match gender {
        ui_caf::Gender::Male => vm_caf::Gender::Male,
        ui_caf::Gender::Female => vm_caf::Gender::Female,
    }
}

const fn to_vm_skin(skin: ui_caf::SkinTone) -> vm_caf::SkinTone {
    match skin {
        ui_caf::SkinTone::Light => vm_caf::SkinTone::Light,
        ui_caf::SkinTone::Medium => vm_caf::SkinTone::Medium,
        ui_caf::SkinTone::Dark => vm_caf::SkinTone::Dark,
    }
}
