//! `sims`: developer tools for the game's data files.

mod census;
mod houses;
mod install;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: sims census <path>...
       sims install <install-root>
       sims houses <install-root> <neighborhood-dir>

  census   Decode every resource under the given files or directories and
           report failures and resource types that have no reader yet.
  install  Detect a game install and list what each mount contributes, in
           the original's load order.
  houses   Load every house in a neighborhood (such as UserData) against an
           install, with the neighborhood's characters registered.";

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
        Some((command, [root])) if command == "install" => match install::report(root.as_ref()) {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Some((command, [root, neighborhood])) if command == "houses" => {
            match houses::report(root.as_ref(), neighborhood.as_ref()) {
                Ok((report, ok)) => {
                    print!("{report}");
                    if ok {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
