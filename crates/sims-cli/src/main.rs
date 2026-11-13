//! `sims`: developer tools for the game's data files.

mod census;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: sims census <path>...

  census   Decode every resource under the given files or directories and
           report failures and resource types that have no reader yet.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((command, paths)) if command == "census" && !paths.is_empty() => {
            let mut census = census::Census::default();
            for path in paths {
                if let Err(error) = census.add_path(&PathBuf::from(path)) {
                    eprintln!("{path}: {error}");
                    return ExitCode::FAILURE;
                }
            }
            print!("{}", census.report());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
