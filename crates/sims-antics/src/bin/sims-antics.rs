//! CLI entrypoint for compiling and validating clean-room `open-pack/` asset trees.
//!
//! Usage:
//! - `sims-antics check <pack_dir>`
//! - `sims-antics build <pack_dir> --out <dir>`

use sims_antics::compile_pack_dir;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn print_usage() {
    eprintln!("Usage:\n  sims-antics check <pack_dir>\n  sims-antics build <pack_dir> --out <dir>");
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        print_usage();
        return Ok(());
    }

    match args[0].as_str() {
        "check" => {
            if args.len() != 2 {
                print_usage();
                return Err("check expects exactly one <pack_dir> argument".to_owned());
            }
            let pack_dir = Path::new(&args[1]);
            let compiled = compile_pack_dir(pack_dir).map_err(|e| e.to_string())?;
            let total_bhavs: usize = compiled
                .objects
                .iter()
                .map(|o| o.object.behaviors.len())
                .sum();
            println!(
                "OK: {} v{} ({} objects, {} BHAVs, {} bytes {}, {} WAVs)",
                compiled.name,
                compiled.version,
                compiled.objects.len(),
                total_bhavs,
                compiled.far_bytes.len(),
                compiled.archive_name,
                compiled.sounds_wav.len()
            );
            Ok(())
        }
        "build" => {
            let mut pack_dir: Option<PathBuf> = None;
            let mut out_dir: Option<PathBuf> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--out" | "-o" => {
                        let val = args
                            .get(i + 1)
                            .ok_or_else(|| "missing directory path after --out".to_owned())?;
                        out_dir = Some(PathBuf::from(val));
                        i += 2;
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag '{other}'"));
                    }
                    other => {
                        if pack_dir.is_some() {
                            return Err(format!("unexpected extra argument '{other}'"));
                        }
                        pack_dir = Some(PathBuf::from(other));
                        i += 1;
                    }
                }
            }
            let pack_dir = pack_dir.ok_or_else(|| "build requires <pack_dir>".to_owned())?;
            let out_dir = out_dir.ok_or_else(|| "build requires --out <dir>".to_owned())?;

            let compiled = compile_pack_dir(&pack_dir).map_err(|e| e.to_string())?;
            let objects_out = out_dir.join("objects");
            let sounds_out = out_dir.join("sounds");
            std::fs::create_dir_all(&objects_out)
                .map_err(|e| format!("failed to create {}: {e}", objects_out.display()))?;
            std::fs::create_dir_all(&sounds_out)
                .map_err(|e| format!("failed to create {}: {e}", sounds_out.display()))?;

            let far_path = out_dir.join(&compiled.archive_name);
            std::fs::write(&far_path, &compiled.far_bytes)
                .map_err(|e| format!("failed to write {}: {e}", far_path.display()))?;

            for entry in &compiled.objects {
                let iff_path = objects_out.join(&entry.iff_name);
                std::fs::write(&iff_path, &entry.object.iff_bytes)
                    .map_err(|e| format!("failed to write {}: {e}", iff_path.display()))?;
            }
            for (name, wav) in &compiled.sounds_wav {
                let wav_path = sounds_out.join(format!("{name}.wav"));
                std::fs::write(&wav_path, wav)
                    .map_err(|e| format!("failed to write {}: {e}", wav_path.display()))?;
            }

            println!(
                "Built {} v{} -> {} ({} objects, {} bytes FAR, {} WAVs)",
                compiled.name,
                compiled.version,
                out_dir.display(),
                compiled.objects.len(),
                compiled.far_bytes.len(),
                compiled.sounds_wav.len()
            );
            Ok(())
        }
        other => {
            print_usage();
            Err(format!("unknown subcommand '{other}'"))
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
