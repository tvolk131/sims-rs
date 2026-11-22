//! CLI entrypoint for compiling and validating clean-room `open-pack/` asset trees.
//!
//! Usage:
//! - `sims-antics check <pack_dir>`
//! - `sims-antics build <pack_dir> --out <dir>`

use sims_antics::compile_pack_dir;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn print_usage() {
    eprintln!("Usage:\n  sims-antics check <pack_dir>\n  sims-antics build <pack_dir> --out <dir>");
}

fn is_macos_system_symlink(path: &Path) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    if path.components().count() != 2 {
        return false;
    }
    std::fs::canonicalize(path)
        .map(|canon| canon.starts_with("/private/"))
        .unwrap_or(false)
}

/// Ensures `path` exists as a real directory by walking every component one step at a time,
/// refusing to traverse or create through any symlink ancestor (except macOS's top-level
/// `/var` / `/tmp` -> `/private/*` system firmlinks), and verifying containment under
/// `expected_parent_canon` when provided.
fn ensure_real_dir(path: &Path, expected_parent_canon: Option<&Path>) -> Result<PathBuf, String> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("failed to resolve current directory: {e}"))?
            .join(path)
    };

    let mut cur = PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::Prefix(p) => {
                cur.push(p.as_os_str());
                continue;
            }
            Component::RootDir => {
                cur.push(comp.as_os_str());
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                if !cur.pop() {
                    return Err(format!("invalid output path {}", path.display()));
                }
                continue;
            }
            Component::Normal(part) => cur.push(part),
        }

        match std::fs::symlink_metadata(&cur) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    if !is_macos_system_symlink(&cur) {
                        return Err(format!(
                            "refusing to write through symlink path {}",
                            cur.display()
                        ));
                    }
                } else if !meta.file_type().is_dir() {
                    return Err(format!(
                        "refusing to write through non-directory path {}",
                        cur.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur)
                    .map_err(|err| format!("failed to create {}: {err}", cur.display()))?;
            }
            Err(e) => {
                return Err(format!("failed to inspect {}: {e}", cur.display()));
            }
        }
    }

    let final_meta = std::fs::symlink_metadata(&cur)
        .map_err(|e| format!("failed to inspect {}: {e}", cur.display()))?;
    if final_meta.file_type().is_symlink() || !final_meta.file_type().is_dir() {
        return Err(format!(
            "refusing to write through non-directory or symlink path {}",
            cur.display()
        ));
    }

    let canon = std::fs::canonicalize(&cur)
        .map_err(|e| format!("failed to resolve {}: {e}", cur.display()))?;
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

/// Writes `bytes` into `parent_canon/<filename>` via an exclusive temp file (`O_CREAT | O_EXCL`)
/// followed by atomic rename, refusing to create or overwrite any symlink or non-regular file.
fn write_contained_file(parent_canon: &Path, filename: &str, bytes: &[u8]) -> Result<(), String> {
    let mut comps = Path::new(filename).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => {
            return Err(format!("invalid output filename '{filename}'"));
        }
    }

    let dest = parent_canon.join(filename);
    if let Ok(meta) = std::fs::symlink_metadata(&dest)
        && (meta.file_type().is_symlink() || !meta.file_type().is_file())
    {
        return Err(format!(
            "refusing to overwrite symlink or non-regular file {}",
            dest.display()
        ));
    }

    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".sims-antics-tmp-{}-{seq}", std::process::id());
    let tmp_path = parent_canon.join(tmp_name);
    let mut tmp_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|e| format!("failed to create temp file {}: {e}", tmp_path.display()))?;
    if let Err(e) = tmp_file.write_all(bytes) {
        drop(tmp_file);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("failed to write {}: {e}", tmp_path.display()));
    }
    drop(tmp_file);

    // Re-check destination immediately before atomic rename (rename itself never follows a
    // destination symlink target, and this rejects pre-existing symlinks/directories cleanly).
    if let Ok(meta) = std::fs::symlink_metadata(&dest)
        && (meta.file_type().is_symlink() || !meta.file_type().is_file())
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "refusing to overwrite symlink or non-regular file {}",
            dest.display()
        ));
    }
    if let Err(e) = std::fs::rename(&tmp_path, &dest) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("failed to commit {}: {e}", dest.display()));
    }
    Ok(())
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
