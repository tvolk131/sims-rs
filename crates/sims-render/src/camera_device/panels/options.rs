use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Section {
    Graphics,
    Sound,
    #[default]
    Play,
}
impl Section {
    pub const ALL: [Self; 3] = [Self::Graphics, Self::Sound, Self::Play];
    pub fn resource(self) -> i32 {
        match self {
            Self::Graphics => 4957,
            Self::Sound => 4958,
            Self::Play => 4959,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Graphics => "Graphics",
            Self::Sound => "Sound",
            Self::Play => "Play",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    pub floor: i32,
    pub zoom: u32,
    pub rotation: u8,
    pub free_will: Option<bool>,
}
impl Sources {
    fn capture(session: &Session, controls: super::super::Ui) -> Self {
        Self { floor: controls.floor, zoom: controls.zoom,
            rotation: session.simulation.vm.world.camera_rotation,
            free_will: session.simulation.vm.world.person.as_ref().map(|person| person.free_will) }
    }
    fn validate(&self) -> Result<()> {
        ensure!(self.floor > 0 && (1..=3).contains(&self.zoom) && self.rotation < 4,
            "Options camera source is invalid");
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub revision: u64,
    pub section: Section,
    pub sources: Sources,
    pub mode: i32,
    pub paused: u8,
    pub window_paused: u8,
    pub pause_overlay: bool,
    pub speed: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    sources: Sources,
    section: Section,
    revision: u64,
    view: Option<View>,
}
impl Options {
    pub(super) fn new(sources: Sources) -> Result<Self> {
        sources.validate()?;
        Ok(Self { sources, section: Section::Play, revision: 0, view: None })
    }
    pub fn view(&self) -> Option<&View> {
        self.view.as_ref()
    }
    pub(super) fn validate(&self) -> Result<()> {
        self.sources.validate()?;
        if let Some(view) = &self.view {
            view.sources.validate()?;
            ensure!(view.mode == 3 && view.section == self.section
                && view.revision == self.revision && self.revision != 0,
                "Options content lost its retained owner");
        }
        Ok(())
    }
    pub(super) fn refresh(&mut self, state: &State, window: &mode::Window) -> Result<()> {
        ensure!(state.mode == 3, "Options refresh outside application mode3");
        self.sources.validate()?;
        let mut view = View { revision: self.revision, section: self.section, sources: self.sources.clone(),
            mode: state.mode, paused: state.paused, window_paused: window.fields().paused,
            pause_overlay: window.indicator_visible(), speed: state.speed };
        if self.view.as_ref() != Some(&view) {
            self.revision = self.revision.checked_add(1).context("Options revision exhausted")?;
            view.revision = self.revision;
            self.view = Some(view);
        }
        self.validate()
    }
    fn select(&mut self, revision: u64, section: Section) -> Result<bool> {
        if self.view.as_ref().is_none_or(|view| view.revision != revision) { return Ok(false); }
        if self.section != section {
            self.revision = self.revision.checked_add(1).context("Options revision exhausted")?;
            self.section = section;
            let view = self.view.as_mut().unwrap();
            view.section = section;
            view.revision = self.revision;
        }
        self.validate()?;
        Ok(true)
    }
}

impl Device {
    pub fn bind_panel_options(&mut self, session: &Session) -> Result<()> {
        validate(session, &self.saved)?;
        let options = Options::new(Sources::capture(session, self.controls()))?;
        let widgets = self.saved.panel_widgets.as_mut().context("Options panel widgets absent")?;
        ensure!(widgets.presentation.is_some() && widgets.options.is_none(), "Options owner absent or already bound");
        widgets.options = Some(options);
        validate(session, &self.saved)
    }
    pub(super) fn refresh_options_sources(&mut self, session: &Session) -> Result<()> {
        let sources = Sources::capture(session, self.controls());
        if let Some(options) = self.saved.panel_widgets.as_mut().and_then(|widgets| widgets.options.as_mut()) {
            sources.validate()?;
            options.sources = sources;
        }
        Ok(())
    }
    pub fn select_options_section(&mut self, session: &Session, revision: u64, section: Section) -> Result<bool> {
        validate(session, &self.saved)?;
        if session.player_input_pending() || session.simulation.vm.world.dialogs.modal.is_some()
            || session.host.presentation.transport.as_ref().is_none_or(|transport| transport.controls.mode != 3) {
            return Ok(false);
        }
        let Some(widgets) = self.saved.panel_widgets.as_mut() else { return Ok(false) };
        if widgets.presentation.as_ref().is_none_or(|presentation| !presentation.visible(Panel::Options)) {
            return Ok(false);
        }
        let Some(options) = &mut widgets.options else { return Ok(false) };
        if !options.select(revision, section)? { return Ok(false); }
        widgets.dirty.insert(widgets.owners.panels[3]);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tutorial_panel_options_show_refreshes_real_snapshot_and_persists_sections() -> Result<()> {
        let sources = Sources { floor: 1, zoom: 2, rotation: 1, free_will: Some(true) };
        let mut options = Options::new(sources)?;
        let mut state = State::constructed(760);
        let window = mode::Window::new(9000, Some(9001), 80, 1, 0);
        ensure!(options.view().is_none());
        options.refresh(&state, &window)?;
        let original = options.view().unwrap().clone();
        ensure!(original.mode == 3 && original.window_paused == 1 && original.sources.free_will == Some(true));
        ensure!(options.select(original.revision, Section::Graphics)?);
        ensure!(!options.select(original.revision, Section::Sound)?);
        let restored: Options = serde_json::from_slice(&serde_json::to_vec(&options)?)?;
        restored.validate()?;
        ensure!(restored == options && restored.view().unwrap().section == Section::Graphics);
        let mut corrupt = serde_json::to_value(&restored)?;
        corrupt["view"]["revision"] = serde_json::json!(0);
        ensure!(serde_json::from_value::<Options>(corrupt)?.validate().is_err());
        options.sources.zoom = 3;
        options.refresh(&state, &window)?;
        ensure!(options.view().unwrap().sources.zoom == 3 && options.view().unwrap().section == Section::Graphics);
        state.mode = 2;
        ensure!(options.refresh(&state, &window).is_err());
        ensure!(Options::new(Sources { floor: 0, zoom: 2, rotation: 0, free_will: None }).is_err());
        Ok(())
    }
}
