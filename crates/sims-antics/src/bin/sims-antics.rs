//! CLI entrypoint for compiling and validating clean-room `open-pack/` asset trees.
//!
//! Usage:
//! - `sims-antics check <pack_dir>`
//! - `sims-antics build <pack_dir> --out <dir>`

use sims_antics::compile_pack_dir;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn print_usage() {
    eprintln!("Usage:\n  sims-antics check <pack_dir>\n  sims-antics build <pack_dir> --out <dir>");
}

fn ensure_real_dir(path: &Path, expected_parent_canon: Option<&Path>) -> Result<PathBuf, String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return Err(format!(
                "refusing to write through non-directory or symlink path {}",
                path.display()
            ));
        }
    } else {
        std::fs::create_dir_all(path)
            .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    }
    let canon = std::fs::canonicalize(path)
        .map_err(|e| format!("failed to resolve {}: {e}", path.display()))?;
    if let Some(parent) = expected_parent_canon
        && !canon.starts_with(parent)
    {
        return Err(format!(
            "output directory {} escapes root {}",
            canon.display(),
            parent.display()
        ));
    }
    Ok(canon)
}

fn write_contained_file(parent_canon: &Path, filename: &str, bytes: &[u8]) -> Result<(), String> {
    let path = parent_canon.join(filename);
    if let Ok(meta) = std::fs::symlink_metadata(&path)
        && (meta.file_type().is_symlink() || !meta.file_type().is_file())
    {
        return Err(format!(
            "refusing to overwrite symlink or non-regular file {}",
            path.display()
        ));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
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
            let out_canon = ensure_real_dir(&out_dir, None)?;
            let objects_canon = ensure_real_dir(&out_canon.join("objects"), Some(&out_canon))?;
            let sounds_canon = ensure_real_dir(&out_canon.join("sounds"), Some(&out_canon))?;

            write_contained_file(&out_canon, &compiled.archive_name, &compiled.far_bytes)?;

            for entry in &compiled.objects {
                write_contained_file(&objects_canon, &entry.iff_name, &entry.object.iff_bytes)?;
            }
            for (name, wav) in &compiled.sounds_wav {
                let wav_name = format!("{name}.wav");
                write_contained_file(&sounds_canon, &wav_name, wav)?;
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
