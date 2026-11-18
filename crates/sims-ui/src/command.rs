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
        assert!(!Command::ShowRelations(Relations::Family).changes_simulation());
        assert!(!Command::RelationsPage(1).changes_simulation());
        assert!(!Command::Help(crate::balloon::Help::Sign(2)).changes_simulation());
    }
}
