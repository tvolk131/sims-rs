//! Persistent original-house presentation owner. Resource services remain
//! synchronous with Dialog; bounded work does not advance the household clock.
use super::{Pending, Progress};
use crate::{
    Camera,
    portrait::{
        cache::{self, Images, Settings},
        images,
        resize::Format,
    },
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::{
    object_catalog::Catalog,
    object_resources::{Checkpoint, Overlay},
};
use sims_runtime::original_session::Session;
use sims_sim::{
    Step, StepStop, placement::SearchView, portrait_person::Service, pose::portrait::view::Input,
};
use std::sync::Arc;
pub mod buttons;
pub mod live_panel;
use buttons::{Buttons, Lookup};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub view: Input,
    pub search: SearchView,
    pub projected: [i32; 2],
}
impl Inputs {
    /// Invert the displayed camera at viewport center on the ground plane.
    /// The portrait surface uses the nearest native discrete pixel density,
    /// centered on the same world point. These are presentation-host inputs,
    /// not a replacement for the original Find Location search rules.
    pub fn camera(camera: Camera, size: i32) -> Result<Self> {
        ensure!(
            camera.width > 0
                && camera.height > 0
                && camera.zoom.is_finite()
                && camera.zoom > 0.
                && camera.center.is_finite()
                && camera.rotation < 4,
            "invalid dialog camera"
        );
        let sum = (-0.07 * camera.height as f32 / camera.zoom - camera.center.y * 78.38367) / 32.;
        let rotated = glam::Vec3::new(sum * 0.5, -camera.center.y, sum * 0.5);
        let inverse = Camera {
            rotation: (4 - camera.rotation) % 4,
            ..camera
        };
        let point = inverse.rotated(rotated) + camera.center;
        let physical = [
            (point.z * 16.).round() as i32,
            (point.x * 16.).round() as i32,
        ];
        let [x, y] = physical;
        let projected = match camera.rotation {
            0 => [x, y],
            1 => [y, 1023 - x],
            2 => [1023 - x, 1023 - y],
            _ => [1023 - y, x],
        };
        let zoom = (1..=3)
            .min_by(|a, b| {
                ((8 << a) as f32 - 64. * camera.zoom)
                    .abs()
                    .total_cmp(&((8 << b) as f32 - 64. * camera.zoom).abs())
            })
            .unwrap();
        let mut view = Input {
            viewport: [0, 0, camera.width, camera.height],
            center: [0, 0],
            zoom,
            rotation: u32::from(camera.rotation),
        };
        let native = [point.z * 3., 0., point.x * 3.];
        let project = |center| -> Result<[f32; 3]> {
            Ok(Input { center, ..view }
                .build()
                .map_err(anyhow::Error::msg)?
                .screen
                .point(native)
                .map_err(anyhow::Error::msg)?)
        };
        let base = project([0, 0])?;
        let a = project([128, 0])?;
        let b = project([0, 128])?;
        let (ax, ay, bx, by) = (
            (a[0] - base[0]) / 128.,
            (a[1] - base[1]) / 128.,
            (b[0] - base[0]) / 128.,
            (b[1] - base[1]) / 128.,
        );
        let det = ax * by - ay * bx;
        ensure!(
            det.is_finite() && det.abs() > 0.1,
            "invalid native portrait camera basis"
        );
        let dx = camera.width as f32 * 0.5 - base[0];
        let dy = camera.height as f32 * 0.5 - base[1];
        view.center = [
            ((dx * by - dy * bx) / det).round() as i32,
            ((dy * ax - dx * ay) / det).round() as i32,
        ];
        view.build().map_err(anyhow::Error::msg)?;
        Ok(Self {
            view,
            search: SearchView {
                viewport: [0, 0, camera.width as i32, camera.height as i32],
                bounds: [0, size, 0, size],
                rotation: i32::from(camera.rotation),
            },
            projected,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Active {
    pending: Pending,
    inputs: Inputs,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    version: u32,
    source: String,
    identity: String,
    resources: Checkpoint,
    images: Images,
    settings: Settings,
    background: u32,
    thumbnail: u32,
    active: Option<Active>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    buttons: Option<Buttons>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lookup: Option<Lookup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live_panel: Option<live_panel::Panel>,
    before: Option<sims_runtime::original_session::Saved>,
}
pub struct Client {
    catalog: Catalog<'static>,
    images: Images,
    settings: Settings,
    background: u32,
    thumbnail: u32,
    identity: String,
    source: String,
    active: Option<Active>,
    buttons: Option<Buttons>,
    lookup: Option<Lookup>,
    live_panel: Option<live_panel::Panel>,
    before: Option<sims_runtime::original_session::Saved>,
}
impl Client {
    pub fn new(session: &mut Session) -> Result<Self> {
        let index = Arc::new(session.index().clone());
        let source = session.house.neighborhood_source.clone();
        let (directory, _) = source
            .rsplit_once('/')
            .context("neighborhood directory absent")?;
        let prefix = format!("{directory}/Characters/").to_ascii_lowercase();
        let overlay = Overlay::copy(
            &index,
            index
                .resources
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.locator.to_ascii_lowercase().starts_with(&prefix)
                        && r.locator.to_ascii_lowercase().ends_with(".iff")
                })
                .map(|(i, _)| i),
        )
        .map_err(anyhow::Error::msg)?;
        let catalog = Catalog::household(index.clone(), &source, overlay)?;
        let identity = catalog.identity().map_err(anyhow::Error::msg)?;
        let mut images = Images::default();
        let cache = &mut session.simulation.vm.world.image_cache;
        ensure!(
            cache.entries.is_empty() && cache.refs.iter().all(|r| *r == 0),
            "existing portrait cache requires its saved client owner"
        );
        catalog
            .populate_portraits(cache, true, false, &mut |e, s| images.event(e, s))
            .map_err(anyhow::Error::msg)?;
        let mut templates = std::collections::BTreeMap::new();
        for id in [108, 4600] {
            templates.insert(
                id,
                images::template(&index, id, Format::Rgb565)?
                    .context("portrait UI template absent")?,
            );
        }
        // Recovered default layout; pixel packing/key are this software client's
        // explicit display profile. Persist all inputs with the suspended call.
        let settings = Settings {
            layout: [45, 45, 5, 2, 35, 41],
            format: Format::Rgb565,
            key: 0xf81f,
            templates,
        };
        let background = images.insert(cache, settings.templates[&4600].clone())?;
        let thumbnail = images.insert(cache, settings.templates[&108].clone())?;
        Ok(Self {
            catalog,
            images,
            settings,
            background,
            thumbnail,
            identity,
            source,
            active: None,
            buttons: None,
            lookup: None,
            live_panel: None,
            before: None,
        })
    }
    pub fn pending(&self) -> bool {
        self.active.is_some() || self.lookup.is_some()
    }
    pub fn preview(&self, session: &Session) -> Result<Option<Session>> {
        ensure!(
            self.lookup.is_none(),
            "UI portrait requires its retained device frame"
        );
        self.before
            .as_ref()
            .map(|s| Session::restore(session.index(), s.clone()))
            .transpose()
    }
    pub fn images(&self) -> &Images {
        &self.images
    }
    pub fn collect(&mut self, session: &Session) -> Result<()> {
        self.images
            .collect(&session.simulation.vm.world.image_cache)
    }
    pub fn save(&self, session: &Session) -> Result<Saved> {
        self.validate(session)?;
        Ok(Saved {
            version: 1,
            source: self.source.clone(),
            identity: self.identity.clone(),
            resources: self
                .catalog
                .resources
                .checkpoint()
                .map_err(anyhow::Error::msg)?,
            images: self.images.clone(),
            settings: self.settings.clone(),
            background: self.background,
            thumbnail: self.thumbnail,
            active: self.active.clone(),
            buttons: self.buttons.clone(),
            lookup: self.lookup.clone(),
            live_panel: self.live_panel.clone(),
            before: self.before.clone(),
        })
    }
    fn validate_managed_preview(&self) -> Result<()> {
        ensure!(
            self.before
                .as_ref()
                .is_none_or(|s| s.version == 2 && s.managed.is_some()),
            "finish the pending legacy research portrait before saving a managed household"
        );
        Ok(())
    }
    pub fn save_managed(&self, session: &Session) -> Result<Saved> {
        self.validate_managed_preview()?;
        self.save(session)
    }
    pub fn restore_managed(session: &Session, saved: Saved) -> Result<Self> {
        let client = Self::restore(session, saved)?;
        client.validate_managed_preview()?;
        Ok(client)
    }
    pub fn restore(session: &Session, saved: Saved) -> Result<Self> {
        ensure!(
            saved.version == 1 && saved.source == session.house.neighborhood_source,
            "dialog client source/version changed"
        );
        let mut catalog = Catalog::household(
            Arc::new(session.index().clone()),
            &saved.source,
            saved.resources.overlay.clone(),
        )?;
        ensure!(
            catalog.identity().map_err(anyhow::Error::msg)? == saved.identity,
            "saved dialog catalog changed"
        );
        catalog
            .resources
            .restore_checkpoint(&saved.resources)
            .map_err(anyhow::Error::msg)?;
        let client = Self {
            catalog,
            images: saved.images,
            settings: saved.settings,
            background: saved.background,
            thumbnail: saved.thumbnail,
            identity: saved.identity,
            source: saved.source,
            active: saved.active,
            buttons: saved.buttons,
            lookup: saved.lookup,
            live_panel: saved.live_panel,
            before: saved.before,
        };
        client.validate(session)?;
        Ok(client)
    }
    pub fn validate(&self, session: &Session) -> Result<()> {
        self.images
            .validate(&session.simulation.vm.world.image_cache)?;
        ensure!(
            self.images.get(self.background)?
                == self
                    .settings
                    .templates
                    .get(&4600)
                    .context("portrait background absent")?
                && self.images.get(self.thumbnail)?
                    == self
                        .settings
                        .templates
                        .get(&108)
                        .context("portrait thumbnail absent")?,
            "saved portrait template binding changed"
        );
        ensure!(
            self.active.is_some() == self.before.is_some()
                && !(self.active.is_some() && self.lookup.is_some()),
            "dialog preview owner mismatch"
        );
        self.validate_buttons(session)?;
        if let Some(panel) = &self.live_panel {
            panel.validate(session)?;
        }
        if let Some(lookup) = &self.lookup {
            lookup.validate(session, &self.images)?;
        } else if let Some(active) = &self.active {
            active.inputs.view.build().map_err(anyhow::Error::msg)?;
            active.pending.validate(session, &self.images)?;
        } else {
            let removal_owned_by_ground = session
                .host
                .presentation
                .ground_input
                .as_ref()
                .and_then(|input| input.removal_target())
                .zip(session.simulation.synchronous_removal.as_ref())
                .is_some_and(|(target, removal)| target == removal.target());
            let removal_owned_by_move = session
                .move_tool_removal_target()
                .zip(session.simulation.synchronous_removal.as_ref())
                .is_some_and(|(target, removal)| target == removal.target());
            ensure!(
                session.simulation.portrait_person.is_none()
                    && (session.simulation.synchronous_removal.is_none()
                        || removal_owned_by_ground
                        || removal_owned_by_move),
                "portrait operation lost its dialog owner"
            );
        }
        Ok(())
    }
    pub fn begin(
        &mut self,
        session: &Session,
        thread: sims_vm::ThreadId,
        inputs: Inputs,
    ) -> Result<()> {
        ensure!(
            !self.pending() && !session.ui_portrait_pending(),
            "dialog construction already active"
        );
        inputs.view.build().map_err(anyhow::Error::msg)?;
        let pending = Pending::begin(
            session,
            &mut self.catalog,
            thread,
            Some(self.background),
            Some(self.thumbnail),
            u32::from(self.settings.key),
        )?;
        // A persistent preview must be independent too: drawing it after a
        // cold player restore must not activate the original starting house.
        self.before = Some(session.save_managed()?);
        self.active = Some(Active { pending, inputs });
        Ok(())
    }
    /// At most `budget` native BHAV instructions. Services execute at their
    /// original boundaries, and the caller may save after any returned slice.
    pub fn advance(&mut self, session: &mut Session, budget: usize) -> Result<(bool, Step)> {
        self.collect(session)?;
        self.validate(session)?;
        let mut remaining = budget;
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        for _ in 0..1000 {
            session.host.update_contexts(&mut session.simulation)?;
            let active = self.active.as_mut().context("dialog constructor absent")?;
            let inputs = active.inputs;
            let progress = active.pending.advance(
                &mut cache::Session {
                    session,
                    view: inputs.view,
                    source_key: None,
                },
                &mut self.catalog,
                &mut self.images,
                &self.settings,
                &mut remaining,
                &mut step,
            )?;
            match progress {
                Progress::Temporary(service) => self.service_temporary(session, inputs, service)?,
                Progress::Yielded => {}
                Progress::Complete => {
                    self.active = None;
                    self.before = None;
                }
            }
            let serviced = session
                .host
                .service(&mut session.simulation, &mut session.program)?;
            self.collect(session)?;
            if !self.pending() {
                return Ok((true, step));
            }
            if remaining == 0 {
                return Ok((false, step));
            }
            if progress == Progress::Yielded && serviced == 0 {
                anyhow::bail!("dialog constructor yielded with no serviced dependency");
            }
        }
        anyhow::bail!("dialog host service limit exceeded")
    }
    fn service_temporary(
        &mut self,
        session: &mut Session,
        inputs: Inputs,
        service: Service,
    ) -> Result<()> {
        match service {
            Service::Resource { guid } => {
                let resource = self
                    .catalog
                    .portrait_resource(guid)
                    .map_err(anyhow::Error::msg)?;
                let definition = resource
                    .map(|r| self.catalog.portrait_definition(r))
                    .transpose()
                    .map_err(anyhow::Error::msg)?;
                if let Some(d) = &definition {
                    if !session.program.modules.contains_key(&d.source) {
                        let loaded = sims_content::vm::load_program(
                            session.index(),
                            &d.source,
                            &session.program.globals,
                        )?;
                        for (name, module) in loaded.modules {
                            if let Some(old) = session.program.modules.get(&name) {
                                ensure!(
                                    old.content_hash == module.content_hash,
                                    "portrait code resource changed"
                                );
                            } else {
                                session.program.modules.insert(name, module);
                            }
                        }
                    }
                }
                session
                    .simulation
                    .supply_portrait_person_resource(&session.program, resource.zip(definition))
                    .map_err(anyhow::Error::msg)?;
            }
            Service::CameraOrigin => session
                .simulation
                .supply_portrait_person_camera(inputs.search, inputs.projected)
                .map_err(anyhow::Error::msg)?,
        }
        Ok(())
    }
}
