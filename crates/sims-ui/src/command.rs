//! What the player asks for. The interface never changes the simulation:
//! a click becomes a [`Command`], and the host decides what to do with it.
//!
//! Commands split in two:
//!
//! - [`Command::changes_simulation`] is true for those only the simulation
//!   can carry out (choosing a Sim, answering a dialog, the speed). They
//!   need a command intake in the simulation, which does not exist yet;
//!   `docs/roadmap.md` lists what it needs.
//! - The others change only how the game is shown (the camera, which
//!   panel is open) and the host applies them itself.

/// The game's speed buttons. The numbers the simulation will use for them
/// are for the intake to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Speed {
    Normal,
    Fast,
    Ultra,
}

/// The modes the cluster's large buttons pick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    #[default]
    Live,
    Buy,
    Build,
    Options,
}

/// The live-mode panels the gadget's buttons open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Panel {
    #[default]
    Needs,
    Interests,
    Relationships,
    Personality,
    Job,
    Gifts,
    House,
}

/// Which people the relationship panel lists: its four buttons, top to
/// bottom (0x51d9aa, 0x4e9deb). The panel starts on everyone (0x51a8c3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relations {
    /// The selected Sim's family.
    Family,
    /// Friends: both ways at least the friendship level.
    Friends,
    /// A Superstar list; disabled without that expansion (0x51a7ea).
    Stars,
    #[default]
    Everyone,
}

/// How walls are drawn: the cluster's three wall buttons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Walls {
    Down,
    #[default]
    Cutaway,
    Up,
}

/// Camera controls: they change only the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Camera {
    RotateLeft,
    RotateRight,
    ZoomIn,
    ZoomOut,
    FloorUp,
    FloorDown,
    /// Show or hide the roof.
    Roof,
    Walls(Walls),
    /// Take a snapshot of the view.
    Photo,
}

/// The options panel's three subpanel tabs (`cWinOptions`, `0x514236..0x514374`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionsTab {
    Graphics,
    Sound,
    Play,
}

/// The three volume sliders in the Sound options subpanel (`0x516624..0x516675`,
/// mapping to audio kinds 37, 38, and 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VolumeChannel {
    Fx,
    Music,
    Vox,
}

/// The 3-way detail radio groups in Graphics options (`0x514d1d..0x514dd7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionsRadio {
    TerrainDetail,
    CharacterDetail,
}

/// Low / Med / High detail levels (`0x5375d5..0x53783b`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DetailLevel {
    Low = 0,
    Med = 1,
    #[default]
    High = 2,
}

/// The boolean checkbox options in the Graphics and Play subpanels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionsToggle {
    AntiAlias,
    Shadows,
    Lighting,
    InterfaceEffects,
    AutoCentering,
    FreeWill,
    EdgeScrolling,
    SimInBackground,
    QuickTips,
    AutoSnapshot,
    LivePip,
    ExportHtml,
}

/// The top-row action buttons and Play subpanel's Reset Tutorial button
/// (`0x514101..0x514235`, `0x515939..0x515a59`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionsAction {
    Neighborhood,
    Save,
    Quit,
    TutorialReset,
}

/// Which Options-panel setting's help balloon to open or toggle
/// (`0x516719..0x51679a`, `0x66c394..0x66c410`, `UIText.iff STR# 145`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionsHelp {
    AntiAlias,
    Shadows,
    Lighting,
    InterfaceEffects,
    TerrainDetail,
    CharacterDetail,
    Fx,
    Music,
    Vox,
    AutoCentering,
    FreeWill,
    EdgeScrolling,
    SimInBackground,
    ResetTutorial,
    QuickTips,
    AutoSnapshot,
    LivePip,
    ExportHtml,
}

/// One player action.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Command {
    /// Make the person with object ID `sim` the selected Sim.
    Select {
        sim: u16,
    },
    SetSpeed(Speed),
    /// Pause, or run again.
    TogglePause,
    /// Go to a mode. Pressing the mode already shown opens or closes its
    /// panel instead ([`Command::TogglePanel`]).
    SetMode(Mode),
    /// Answer the dialog the object `caller` shows with its button
    /// `button` (0 the first, as the dialog lists them).
    AnswerDialog {
        caller: u16,
        button: u8,
    },
    /// Choose entry `interaction` of the pie menu `sim` opened on
    /// `object`.
    ChooseInteraction {
        sim: u16,
        object: u16,
        interaction: u16,
    },
    /// Cancel `sim`'s action numbered `action`, from its icon in the
    /// queue ([`crate::queue`]).
    CancelAction {
        sim: u16,
        action: i32,
    },
    /// Show or hide the control panel's strip (presentation only).
    TogglePanel,
    /// Open a live-mode panel (presentation only).
    OpenPanel(Panel),
    /// A press the panel takes and refuses (presentation only): a gadget
    /// button pressed with nobody selected (0x51be6e), or the House
    /// button with nobody selected or on a lot that is not residential
    /// (0x51be1a). Only the House button's refusal sounds: with `error`,
    /// the host plays `UI_error` for the interface.
    Refused {
        error: bool,
    },
    /// List these people in the relationship panel (presentation only).
    ShowRelations(Relations),
    /// Show this page of the relationship panel, from 0 (presentation
    /// only).
    RelationsPage(u32),
    /// Move the camera (presentation only).
    Camera(Camera),
    /// Open the help balloon `help`, or close it if it is the one open
    /// (presentation only).
    Help(crate::balloon::Help),
    /// Open the Help System dialog from the cluster's `?` button
    /// (`0x53aaad -> 0x50e8d3`, presentation only).
    OpenHelp,
    /// Open Buy Mode top-level category `0..8` (`0x50633e`, presentation only).
    BuyCategory(usize),
    /// Select Buy Mode subcategory `0..8` (`0x50633e`, presentation only).
    BuySubcategory(usize),
    /// Return from Buy Mode subcategories to top-level categories (`0x50633e`,
    /// presentation only).
    BuyBack,
    /// Toggle Buy Mode catalog sort order between By-Function and By-Room
    /// (`0x53a2c3`, `0x4cf3af`, presentation only).
    BuyToggleSort,
    /// Step the Buy Mode catalog grid page by `-1` or `+1` (`0x50633e`,
    /// presentation only).
    BuyPage(i32),
    /// Select catalog master GUID `u32` in the Buy Mode catalog grid
    /// (`0x5065c8`).
    BuyItem(u32),
    /// Undo the most recent Buy/Build action (`0x4cf2e0` → `0x4932fa`).
    Undo,
    /// Redo the most recently undone Buy/Build action (`0x4cf317` → `0x493338`).
    Redo,
    /// Open or toggle an Options panel subpanel tab (`0x5167d5..0x5167fb`).
    OpenOptionsTab(OptionsTab),
    /// Toggle an Options checkbox (`0x5163cb..0x516520`).
    ToggleOption(OptionsToggle),
    /// Set a 3-way detail radio group in Graphics options (`0x516261..0x5162e5`).
    SetDetail {
        radio: OptionsRadio,
        level: DetailLevel,
    },
    /// Set one of the three Sound volume sliders to a step in `0..=10`
    /// (`0x516624..0x516675`, kinds 37–39).
    SetVolume {
        channel: VolumeChannel,
        step: u8,
    },
    /// Press an Options panel action button (Save, Neighborhood, Quit, or
    /// Reset Tutorial).
    OptionsAction(OptionsAction),
    /// Open or toggle an Options setting's help balloon (`0x516719..0x51679a`).
    OptionsHelp(OptionsHelp),
}

impl Command {
    /// Whether only the simulation can carry the command out. The rest
    /// change only what is shown.
    pub fn changes_simulation(&self) -> bool {
        matches!(
            self,
            Self::Select { .. }
                | Self::SetSpeed(_)
                | Self::TogglePause
                | Self::SetMode(_)
                | Self::AnswerDialog { .. }
                | Self::ChooseInteraction { .. }
                | Self::CancelAction { .. }
                | Self::Undo
                | Self::Redo
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_commands_stay_out_of_the_simulation() {
        assert!(Command::Select { sim: 21 }.changes_simulation());
        assert!(Command::SetMode(Mode::Buy).changes_simulation());
        assert!(!Command::Camera(Camera::ZoomIn).changes_simulation());
        assert!(!Command::OpenPanel(Panel::Job).changes_simulation());
        assert!(!Command::TogglePanel.changes_simulation());
        assert!(!Command::Refused { error: true }.changes_simulation());
        assert!(!Command::ShowRelations(Relations::Family).changes_simulation());
        assert!(!Command::RelationsPage(1).changes_simulation());
        assert!(!Command::Help(crate::balloon::Help::Sign(2)).changes_simulation());
        assert!(!Command::OpenHelp.changes_simulation());
    }
}
