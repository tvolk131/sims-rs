//! Iced application entry point; original households and authored labs share support.
use clap::Parser;
use std::path::{Path, PathBuf};
#[path = "../dialog_view.rs"]
mod dialog_view;
#[path = "../laboratory.rs"]
mod laboratory;
#[path = "../original_viewer.rs"]
mod original_viewer;
#[path = "../support.rs"]
mod support;

#[derive(Parser, Clone)]
#[command(
    about = "Play original Sims 1 households or authored test rooms",
    version
)]
struct Args {
    /// Load an original house, e.g. UserData2/Houses/House09.iff.
    #[arg(long, conflicts_with_all=["household","home","domestic","two_floor","queue"])]
    original: Option<String>,
    /// Allow the documented, unverified rectangle routing in original houses.
    #[arg(long, conflicts_with_all=["laboratory","household","home","domestic","two_floor","queue"])]
    inferred_routing: bool,
    /// Import an original process observation when the save has no retained neighborhood input.
    #[arg(long, requires_all=["original","neighborhood_summary"])]
    neighborhood_observation: Option<PathBuf>,
    /// Evidence summary scoping the observation log to this neighborhood and lot.
    #[arg(long, requires = "neighborhood_observation")]
    neighborhood_summary: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    neighborhood_sample: usize,
    #[arg(long, value_parser = ["installed-houses-ascending-v1"], conflicts_with_all = ["neighborhood_observation", "neighborhood_summary", "neighborhood_sample", "laboratory", "household", "home", "domestic", "two_floor", "queue"])]
    neighborhood_policy: Option<String>,
    /// Open the authored two-chair laboratory instead of a real household.
    #[arg(long, conflicts_with = "original")]
    laboratory: bool,
    /// Attach the experimental Buy frontend when starting a fresh original house.
    #[arg(long, conflicts_with_all=["load","laboratory","household","home","domestic","two_floor","queue"])]
    buy_preview: bool,
    #[arg(long, default_value = "game-data/cabinet-extracted")]
    root: PathBuf,
    #[arg(long, default_value = "saves/room-viewer.json")]
    save: PathBuf,
    #[arg(long)]
    load: Option<PathBuf>,
    /// Use the livable household with food, hygiene, rest, reading and cleanup.
    #[arg(long)]
    household: bool,
    /// Two residents sharing a divided, single-story home.
    #[arg(long)]
    home: bool,
    /// Complete domestic fixture with cooking, table meals and single beds.
    #[arg(long)]
    domestic: bool,
    /// Four residents sharing bedrooms upstairs and a kitchen/bathroom downstairs.
    #[arg(long)]
    two_floor: bool,
    #[arg(long, default_value_t = 1, value_parser=clap::value_parser!(i32).range(1..=2))]
    floor: i32,
    #[arg(long,default_value_t=1,value_parser=clap::value_parser!(u8).range(1..=8))]
    resident: u8,
    #[arg(long, default_value_t = 1)]
    facing: u8,
    #[arg(long, default_value_t = 0, value_parser=clap::value_parser!(u8).range(0..4))]
    camera: u8,
    #[arg(long)]
    paused: bool,
    /// Initial tick in authored rooms; additional ticks after loading an original house.
    #[arg(long, default_value_t = 0)]
    at_tick: u32,
    /// Seed the interaction queue with chair numbers, e.g. --queue 1,2,1.
    #[arg(long, value_delimiter = ',', value_parser=clap::value_parser!(u8).range(1..=2))]
    queue: Vec<u8>,
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Render the complete Iced interface offscreen, without opening a window.
    #[arg(long, requires = "screenshot")]
    headless_screenshot: bool,
    #[arg(long)]
    exit_after_screenshot: bool,
    /// Run deterministic headless gameplay exploration described by this JSON plan.
    #[arg(long, requires_all = ["original", "exploration_output"])]
    exploration_plan: Option<PathBuf>,
    /// New directory for isolated checkpoints, commands, failures and optional frames.
    #[arg(long, requires = "exploration_plan")]
    exploration_output: Option<PathBuf>,
    #[arg(long, requires = "original", conflicts_with_all = ["exploration_plan", "screenshot"])]
    window_session_output: Option<PathBuf>,
    #[arg(long, requires = "window_session_output")]
    window_session_present_library: Option<PathBuf>,
}
impl Args {
    fn select_default_house(&mut self) {
        if self.original.is_none()
            && !(self.laboratory
                || self.household
                || self.home
                || self.domestic
                || self.two_floor
                || !self.queue.is_empty())
        {
            self.original = Some("UserData/Houses/House07.iff".into());
        }
    }
}
fn main() -> anyhow::Result<()> {
    let mut args = Args::parse();
    args.select_default_house();
    if args.two_floor {
        args.domestic = true;
    }
    if args.domestic {
        args.home = true;
    }
    if (args.household || args.home) && args.save == Path::new("saves/room-viewer.json") {
        args.save = if args.two_floor {
            "saves/two-floor-home.json"
        } else if args.domestic {
            "saves/domestic-home.json"
        } else if args.home {
            "saves/two-sim-home.json"
        } else {
            "saves/household-viewer.json"
        }
        .into();
    }
    // Finder supplies an unrelated cwd. Use explicit workspace paths for the
    // development bundle: getcwd() can block while macOS resolves access to its
    // parent Desktop directory, even before an asset has been opened.
    if args.root == Path::new("game-data/cabinet-extracted")
        && let Ok(exe) = std::env::current_exe()
        && exe
            .ancestors()
            .any(|p| p.extension().is_some_and(|e| e == "app"))
        && let Some(workspace) = exe.ancestors().find(|p| p.join(&args.root).is_dir())
    {
        args.root = workspace.join(&args.root);
        if args.save.is_relative() {
            args.save = workspace.join(&args.save);
        }
        for path in [&mut args.load, &mut args.screenshot].into_iter().flatten() {
            if path.is_relative() {
                *path = workspace.join(&*path);
            }
        }
    }
    if args.original.is_some() {
        return original_viewer::run(args);
    }
    laboratory::run(args)
}
