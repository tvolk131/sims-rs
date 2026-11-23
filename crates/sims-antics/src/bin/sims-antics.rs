//! CLI entrypoint for compiling, verifying, checking equivalence, and inspecting
//! lifted `.antics` behavior trees from `.iff` files and `.far` archives.
//!
//! Usage:
//! - `sims-antics check <pack_dir>`
//! - `sims-antics verify <pack_dir>`
//! - `sims-antics equiv <a.antics> <b.iff> --tree <id>`
//! - `sims-antics lift <file.iff|archive.far> [--entry <name.iff>] [--tree <id>] [--list] [--filter <substr>] [--raw] [--verify-equiv]`
//! - `sims-antics build <pack_dir> --out <dir>`

use sims_antics::{
    EquivOptions, EquivResult, StructuredLiftOutcome, check_bhav_equivalent_with_arities,
    compile_antics, compile_pack_dir, compile_standalone_pack_dir,
    lift::{format_bhav_antics, lift_bhav},
    lift_bhav_structured,
    verify::{extract_bhav_arities_from_iff, extract_bhav_from_iff},
    verify_pack_safety,
};
use sims_formats::far;
use sims_formats::iff::{DELETED, Iff};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

enum CliError {
    Message(String),
    BrokenPipe,
}

impl From<String> for CliError {
    fn from(msg: String) -> Self {
        Self::Message(msg)
    }
}

struct StdoutOut {
    writer: BufWriter<std::io::StdoutLock<'static>>,
}

impl StdoutOut {
    fn new() -> Self {
        Self {
            writer: BufWriter::new(std::io::stdout().lock()),
        }
    }

    fn map_io(err: std::io::Error) -> CliError {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            CliError::BrokenPipe
        } else {
            CliError::Message(format!("failed writing to stdout: {err}"))
        }
    }

    fn write_str(&mut self, s: &str) -> Result<(), CliError> {
        self.writer.write_all(s.as_bytes()).map_err(Self::map_io)
    }

    fn writeln_str(&mut self, s: &str) -> Result<(), CliError> {
        self.writer.write_all(s.as_bytes()).map_err(Self::map_io)?;
        self.writer.write_all(b"\n").map_err(Self::map_io)
    }

    fn flush(&mut self) -> Result<(), CliError> {
        self.writer.flush().map_err(Self::map_io)
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         sims-antics check <pack_dir>\n  \
         sims-antics verify <pack_dir>\n  \
         sims-antics equiv <a.antics> <b.iff> --tree <id>\n  \
         sims-antics lift <file.iff|archive.far> [--entry <name.iff>] [--tree <id>] [--list] [--filter <substr>] [--raw] [--verify-equiv]\n  \
         sims-antics build <pack_dir> --out <dir>"
    );
}

fn parse_tree_id(val: &str) -> Result<u16, String> {
    if let Some(hex) = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16)
    } else {
        val.parse::<u16>()
    }
    .map_err(|_| format!("invalid --tree ID '{val}'"))
}

fn sanitize_single_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn normalize_archive_path(s: &str) -> String {
    s.replace('\\', "/")
}

fn is_iff_entry_name(name: &str) -> bool {
    name.as_bytes()
        .last_chunk::<4>()
        .is_some_and(|tail| tail.eq_ignore_ascii_case(b".iff"))
}

fn format_ambiguous_matches(query: &str, matches: &[&far::Entry]) -> String {
    const MAX_LISTED: usize = 20;
    let mut names: Vec<String> = matches
        .iter()
        .map(|e| sanitize_single_line(&e.name))
        .collect();
    names.sort();
    let mut msg = format!(
        "ambiguous --entry '{query}' matched {} FAR entries:",
        names.len()
    );
    for name in names.iter().take(MAX_LISTED) {
        msg.push_str("\n  ");
        msg.push_str(name);
    }
    if names.len() > MAX_LISTED {
        msg.push_str(&format!("\n  ... and {} more", names.len() - MAX_LISTED));
    }
    msg
}

fn resolve_far_entry<'a>(
    entries: &'a [far::Entry],
    query: &str,
    archive_path: &Path,
) -> Result<&'a far::Entry, String> {
    // 1. Exact match.
    let exact: Vec<&far::Entry> = entries.iter().filter(|e| e.name == query).collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    if exact.len() > 1 {
        return Err(format_ambiguous_matches(query, &exact));
    }

    // 2. Case-insensitive exact match (normalizing `\` and `/`).
    let q_norm = normalize_archive_path(query);
    let ci_exact: Vec<&far::Entry> = entries
        .iter()
        .filter(|e| normalize_archive_path(&e.name).eq_ignore_ascii_case(&q_norm))
        .collect();
    if ci_exact.len() == 1 {
        return Ok(ci_exact[0]);
    }
    if ci_exact.len() > 1 {
        return Err(format_ambiguous_matches(query, &ci_exact));
    }

    // 3. Unique path-segment suffix / basename match (case-insensitive, separator-normalized).
    let q_trimmed = q_norm.trim_start_matches('/');
    if !q_trimmed.is_empty() {
        let q_lower = q_trimmed.to_ascii_lowercase();
        let seg_suffix: Vec<&far::Entry> = entries
            .iter()
            .filter(|e| {
                let norm = normalize_archive_path(&e.name).to_ascii_lowercase();
                if norm == q_lower {
                    true
                } else if norm.len() > q_lower.len() && norm.ends_with(&q_lower) {
                    norm.as_bytes()[norm.len() - q_lower.len() - 1] == b'/'
                } else {
                    false
                }
            })
            .collect();
        if seg_suffix.len() == 1 {
            return Ok(seg_suffix[0]);
        }
        if seg_suffix.len() > 1 {
            return Err(format_ambiguous_matches(query, &seg_suffix));
        }

        // 4. Unique general case-insensitive suffix fallback.
        let general_suffix: Vec<&far::Entry> = entries
            .iter()
            .filter(|e| {
                normalize_archive_path(&e.name)
                    .to_ascii_lowercase()
                    .ends_with(&q_lower)
            })
            .collect();
        if general_suffix.len() == 1 {
            return Ok(general_suffix[0]);
        }
        if general_suffix.len() > 1 {
            return Err(format_ambiguous_matches(query, &general_suffix));
        }
    }

    Err(format!(
        "FAR entry '{query}' not found in {} (use --list to list .iff entries)",
        archive_path.display()
    ))
}

fn lift_reason(outcome: &StructuredLiftOutcome) -> Option<String> {
    match outcome {
        StructuredLiftOutcome::Lifted(_) => None,
        StructuredLiftOutcome::UnsupportedOpcode(op) => {
            Some(format!("unsupported opcode {op:#06x}"))
        }
        StructuredLiftOutcome::UnsupportedHeaderOrTarget => {
            Some("unsupported header or target".to_owned())
        }
        StructuredLiftOutcome::Unstructured => Some("unstructured CFG".to_owned()),
    }
}

fn bhav_matches_filter(id: u16, label: &str, filter_lower: &str) -> bool {
    if label.to_ascii_lowercase().contains(filter_lower) {
        return true;
    }
    let dec = id.to_string();
    if dec.contains(filter_lower) {
        return true;
    }
    let hex = format!("{id:#06x}");
    if hex.contains(filter_lower) {
        return true;
    }
    let hex_short = format!("{id:#x}");
    hex_short.contains(filter_lower)
}

fn read_regular_file_prefix(path: &Path) -> Result<(std::fs::File, u64, Vec<u8>), String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("failed to stat {}: {e}", path.display()))?;
    if !meta.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let file_len = meta.len();
    let mut header = Vec::with_capacity(16);
    (&mut file)
        .take(16)
        .read_to_end(&mut header)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    Ok((file, file_len, header))
}

fn read_bounded_iff_file(
    path: &Path,
    mut file: std::fs::File,
    file_len: u64,
    header: Vec<u8>,
) -> Result<Vec<u8>, String> {
    if file_len > far::MAX_MEMBER {
        return Err(format!(
            "{} exceeds maximum IFF size ({} bytes > {} bytes)",
            path.display(),
            file_len,
            far::MAX_MEMBER
        ));
    }
    let mut bytes = Vec::with_capacity(file_len as usize);
    bytes.extend_from_slice(&header);
    let max_rest = far::MAX_MEMBER.saturating_sub(header.len() as u64);
    (&mut file)
        .take(max_rest.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    if bytes.len() as u64 > far::MAX_MEMBER {
        return Err(format!(
            "{} grew past maximum IFF size ({} bytes)",
            path.display(),
            far::MAX_MEMBER
        ));
    }
    Ok(bytes)
}

fn cmd_lift(args: &[String]) -> Result<(), CliError> {
    let mut input_path: Option<PathBuf> = None;
    let mut entry_name: Option<String> = None;
    let mut tree_id: Option<u16> = None;
    let mut list = false;
    let mut filter: Option<String> = None;
    let mut raw = false;
    let mut verify_equiv = false;
    let mut end_of_flags = false;

    let mut i = 1usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if !end_of_flags {
            match arg {
                "-h" | "--help" => {
                    print_usage();
                    return Ok(());
                }
                "--" => {
                    end_of_flags = true;
                    i += 1;
                    continue;
                }
                "--entry" | "-e" => {
                    if entry_name.is_some() {
                        return Err("duplicate --entry flag".to_owned().into());
                    }
                    let val = args
                        .get(i + 1)
                        .ok_or_else(|| "missing entry name after --entry".to_owned())?;
                    entry_name = Some(val.clone());
                    i += 2;
                    continue;
                }
                "--tree" | "-t" => {
                    if tree_id.is_some() {
                        return Err("duplicate --tree flag".to_owned().into());
                    }
                    let val = args
                        .get(i + 1)
                        .ok_or_else(|| "missing tree ID after --tree".to_owned())?;
                    tree_id = Some(parse_tree_id(val)?);
                    i += 2;
                    continue;
                }
                "--list" | "-l" => {
                    list = true;
                    i += 1;
                    continue;
                }
                "--filter" | "-f" => {
                    if filter.is_some() {
                        return Err("duplicate --filter flag".to_owned().into());
                    }
                    let val = args
                        .get(i + 1)
                        .ok_or_else(|| "missing substring after --filter".to_owned())?;
                    filter = Some(val.clone());
                    i += 2;
                    continue;
                }
                "--raw" => {
                    raw = true;
                    i += 1;
                    continue;
                }
                "--verify-equiv" => {
                    verify_equiv = true;
                    i += 1;
                    continue;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag '{other}'").into());
                }
                _ => {}
            }
        }
        if input_path.is_some() {
            return Err(format!("unexpected extra argument '{arg}'").into());
        }
        input_path = Some(PathBuf::from(arg));
        i += 1;
    }

    let input_path = input_path.ok_or_else(|| {
        print_usage();
        "lift requires <file.iff|archive.far>".to_owned()
    })?;

    if list && (tree_id.is_some() || raw || verify_equiv) {
        return Err(
            "--list cannot be combined with --tree, --raw, or --verify-equiv"
                .to_owned()
                .into(),
        );
    }
    if raw && verify_equiv {
        return Err("--verify-equiv cannot be combined with --raw"
            .to_owned()
            .into());
    }
    if tree_id.is_some() && filter.is_some() {
        return Err("--filter cannot be combined with --tree".to_owned().into());
    }

    let (mut file, file_len, header) = read_regular_file_prefix(&input_path)?;
    let has_far_ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("far"));
    let has_far_magic = header.len() >= 8 && &header[..8] == b"FAR!byAZ";
    let is_far = has_far_ext || has_far_magic;

    let mut out = StdoutOut::new();

    let (iff_bytes, source_label) = if is_far {
        if header.len() < 16 {
            return Err(format!(
                "invalid FAR archive {}: file shorter than 16-byte header",
                input_path.display()
            )
            .into());
        }
        let manifest_off = far::manifest_offset(&header, file_len).map_err(|e| {
            format!(
                "failed to parse FAR header in {}: {e}",
                input_path.display()
            )
        })?;
        let dir_len = far::directory_len(manifest_off, file_len)
            .map_err(|e| format!("invalid FAR directory in {}: {e}", input_path.display()))?;
        file.seek(SeekFrom::Start(u64::from(manifest_off)))
            .map_err(|e| format!("failed to seek {}: {e}", input_path.display()))?;
        let mut dir_buf = vec![0u8; dir_len];
        file.read_exact(&mut dir_buf).map_err(|e| {
            format!(
                "failed to read FAR directory in {}: {e}",
                input_path.display()
            )
        })?;
        let entries = far::directory(&dir_buf, file_len).map_err(|e| {
            format!(
                "failed to parse FAR directory in {}: {e}",
                input_path.display()
            )
        })?;

        match entry_name {
            None => {
                if !list {
                    return Err(format!(
                        "{} is a FAR archive; pass --list to list .iff entries or --entry <name.iff> to inspect one",
                        input_path.display()
                    )
                    .into());
                }
                let filter_lower = filter.as_deref().map(str::to_ascii_lowercase);
                let mut iff_entries: Vec<&far::Entry> = entries
                    .iter()
                    .filter(|e| is_iff_entry_name(&e.name))
                    .filter(|e| match &filter_lower {
                        Some(needle) => e.name.to_ascii_lowercase().contains(needle),
                        None => true,
                    })
                    .collect();
                iff_entries
                    .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.offset.cmp(&b.offset)));
                for entry in iff_entries {
                    out.writeln_str(&sanitize_single_line(&entry.name))?;
                }
                out.flush()?;
                return Ok(());
            }
            Some(ref query) => {
                let entry = resolve_far_entry(&entries, query, &input_path)?;
                if !entry.within_payload(manifest_off) {
                    return Err(format!(
                        "FAR entry '{}' in {} has out-of-bounds payload",
                        sanitize_single_line(&entry.name),
                        input_path.display()
                    )
                    .into());
                }
                if entry.stored_size != entry.unpacked_size {
                    return Err(format!(
                        "compressed FAR entry '{}' ({} stored / {} unpacked) is not supported",
                        sanitize_single_line(&entry.name),
                        entry.stored_size,
                        entry.unpacked_size
                    )
                    .into());
                }
                if u64::from(entry.stored_size) > far::MAX_MEMBER {
                    return Err(format!(
                        "FAR entry '{}' exceeds maximum member size ({} > {})",
                        sanitize_single_line(&entry.name),
                        entry.stored_size,
                        far::MAX_MEMBER
                    )
                    .into());
                }
                file.seek(SeekFrom::Start(u64::from(entry.offset)))
                    .map_err(|e| format!("failed to seek {}: {e}", input_path.display()))?;
                let mut member_bytes = vec![0u8; entry.stored_size as usize];
                file.read_exact(&mut member_bytes).map_err(|e| {
                    format!(
                        "failed to read FAR entry '{}' from {}: {e}",
                        sanitize_single_line(&entry.name),
                        input_path.display()
                    )
                })?;
                let label = format!(
                    "{}!{}",
                    input_path.display(),
                    sanitize_single_line(&entry.name)
                );
                (member_bytes, label)
            }
        }
    } else {
        if entry_name.is_some() {
            return Err(format!(
                "--entry is only valid for .far archives ({} is not a FAR archive)",
                input_path.display()
            )
            .into());
        }
        let bytes = read_bounded_iff_file(&input_path, file, file_len, header)?;
        (bytes, input_path.display().to_string())
    };

    let iff =
        Iff::parse(iff_bytes).map_err(|e| format!("failed to parse IFF {source_label}: {e}"))?;

    let mut bhav_chunks: Vec<&sims_formats::iff::Chunk> = iff
        .all(b"BHAV")
        .filter(|c| c.flags & DELETED == 0)
        .collect();
    bhav_chunks.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.flags.cmp(&b.flags))
            .then_with(|| a.offset.cmp(&b.offset))
    });

    if list {
        let filter_lower = filter.as_deref().map(str::to_ascii_lowercase);
        out.writeln_str(&format!(
            "{:<7} {:<8} {:<5} {:<5} {:<7} {:<36} {}",
            "ID", "HEX", "INS", "ARGS", "LOCALS", "STATUS", "NAME"
        ))?;
        for chunk in bhav_chunks {
            if let Some(ref needle) = filter_lower
                && !bhav_matches_filter(chunk.id, &chunk.label, needle)
            {
                continue;
            }
            let safe_name = sanitize_single_line(&chunk.label);
            match lift_bhav(chunk.id, &chunk.label, iff.data(chunk)) {
                Ok(bhav) => {
                    let status = match lift_bhav_structured(&bhav) {
                        StructuredLiftOutcome::Lifted(_) => "structured".to_owned(),
                        other => format!(
                            "raw-only ({})",
                            lift_reason(&other).unwrap_or_else(|| "unknown".to_owned())
                        ),
                    };
                    out.writeln_str(&format!(
                        "{:<7} {:#06x}   {:<5} {:<5} {:<7} {:<36} {}",
                        bhav.id,
                        bhav.id,
                        bhav.instructions.len(),
                        bhav.args,
                        bhav.locals,
                        status,
                        safe_name
                    ))?;
                }
                Err(err) => {
                    let status =
                        format!("unparseable ({})", sanitize_single_line(&err.to_string()));
                    out.writeln_str(&format!(
                        "{:<7} {:#06x}   {:<5} {:<5} {:<7} {:<36} {}",
                        chunk.id, chunk.id, "-", "-", "-", status, safe_name
                    ))?;
                }
            }
        }
        out.flush()?;
        return Ok(());
    }

    let selected_chunks: Vec<&sims_formats::iff::Chunk> = if let Some(target_id) = tree_id {
        let chunk = iff
            .get(b"BHAV", target_id)
            .filter(|c| c.flags & DELETED == 0)
            .or_else(|| bhav_chunks.iter().copied().find(|c| c.id == target_id))
            .ok_or_else(|| {
                format!("BHAV #{target_id} ({target_id:#06x}) not found in {source_label}")
            })?;
        vec![chunk]
    } else if let Some(ref needle_raw) = filter {
        let needle = needle_raw.to_ascii_lowercase();
        let matched: Vec<_> = bhav_chunks
            .into_iter()
            .filter(|c| bhav_matches_filter(c.id, &c.label, &needle))
            .collect();
        if matched.is_empty() {
            return Err(
                format!("no BHAV chunks matched filter '{needle_raw}' in {source_label}").into(),
            );
        }
        matched
    } else {
        bhav_chunks
    };

    if selected_chunks.is_empty() {
        out.writeln_str(&format!("// no BHAV chunks found in {source_label}"))?;
        out.flush()?;
        return Ok(());
    }

    let arities = if verify_equiv {
        Some(extract_bhav_arities_from_iff(&iff.bytes).unwrap_or_default())
    } else {
        None
    };

    let mut counterexamples = 0usize;
    let mut compile_errors = 0usize;

    for (idx, chunk) in selected_chunks.iter().enumerate() {
        if idx > 0 {
            out.writeln_str("")?;
        }
        let bhav = match lift_bhav(chunk.id, &chunk.label, iff.data(chunk)) {
            Ok(b) => b,
            Err(err) => {
                if tree_id.is_some() {
                    return Err(format!(
                        "failed to parse BHAV #{} ({:#06x}) in {source_label}: {err}",
                        chunk.id, chunk.id
                    )
                    .into());
                }
                out.writeln_str(&format!(
                    "// tree {} \"{}\": failed to parse BHAV ({})",
                    chunk.id,
                    sanitize_single_line(&chunk.label),
                    sanitize_single_line(&err.to_string())
                ))?;
                continue;
            }
        };

        if raw {
            let src = format_bhav_antics(&bhav);
            out.write_str(&src)?;
            if !src.ends_with('\n') {
                out.writeln_str("")?;
            }
            continue;
        }

        let outcome = lift_bhav_structured(&bhav);
        match outcome {
            StructuredLiftOutcome::Lifted(src) => {
                if !bhav.label.is_empty() {
                    out.writeln_str(&format!(
                        "// tree {} \"{}\"",
                        bhav.id,
                        sanitize_single_line(&bhav.label)
                    ))?;
                }
                out.write_str(&src)?;
                if !src.ends_with('\n') {
                    out.writeln_str("")?;
                }
                if verify_equiv {
                    match compile_antics(&src) {
                        Ok(compiled) => {
                            if let Some(recompiled) = compiled.behaviors.get(&bhav.id) {
                                match check_bhav_equivalent_with_arities(
                                    &bhav,
                                    recompiled,
                                    &EquivOptions::default(),
                                    arities.as_ref(),
                                ) {
                                    EquivResult::Equivalent {
                                        states_checked,
                                        paths_verified,
                                    } => {
                                        out.writeln_str(&format!(
                                            "// verify-equiv: Equivalent ({states_checked} states checked, {paths_verified} paths verified)"
                                        ))?;
                                    }
                                    EquivResult::Counterexample { path, divergence } => {
                                        counterexamples += 1;
                                        out.writeln_str(&format!(
                                            "// verify-equiv: Counterexample(after {} step(s): {})",
                                            path.len(),
                                            sanitize_single_line(&divergence)
                                        ))?;
                                    }
                                    EquivResult::Inconclusive { reason } => {
                                        out.writeln_str(&format!(
                                            "// verify-equiv: Inconclusive({})",
                                            sanitize_single_line(&reason)
                                        ))?;
                                    }
                                }
                            } else {
                                compile_errors += 1;
                                out.writeln_str(&format!(
                                    "// verify-equiv: CompileError(recompiled output missing tree #{})",
                                    bhav.id
                                ))?;
                            }
                        }
                        Err(err) => {
                            compile_errors += 1;
                            out.writeln_str(&format!(
                                "// verify-equiv: CompileError({})",
                                sanitize_single_line(&err.to_string())
                            ))?;
                        }
                    }
                }
            }
            other => {
                let reason = lift_reason(&other).unwrap_or_else(|| "unknown".to_owned());
                out.writeln_str(&format!(
                    "// tree {} \"{}\": structured lift fallback ({reason}); showing raw low-level .antics",
                    bhav.id,
                    sanitize_single_line(&bhav.label)
                ))?;
                let raw_src = format_bhav_antics(&bhav);
                out.write_str(&raw_src)?;
                if !raw_src.ends_with('\n') {
                    out.writeln_str("")?;
                }
            }
        }
    }

    out.flush()?;

    if counterexamples > 0 || compile_errors > 0 {
        return Err(format!(
            "verify-equiv failed ({counterexamples} counterexample(s), {compile_errors} compile error(s))"
        )
        .into());
    }
    Ok(())
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
/// followed by atomic rename, refusing to create or overwrite any symlink or non-regular file and
/// verifying parent directory identity `(dev, ino)` before commit.
fn write_contained_file(parent_canon: &Path, filename: &str, bytes: &[u8]) -> Result<(), String> {
    let mut comps = Path::new(filename).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => {
            return Err(format!("invalid output filename '{filename}'"));
        }
    }

    let parent_pin = std::fs::File::open(parent_canon).map_err(|e| {
        format!(
            "failed to pin output directory {}: {e}",
            parent_canon.display()
        )
    })?;
    let parent_meta = parent_pin.metadata().map_err(|e| {
        format!(
            "failed to stat output directory {}: {e}",
            parent_canon.display()
        )
    })?;
    if !parent_meta.file_type().is_dir() {
        return Err(format!(
            "output parent {} is not a directory",
            parent_canon.display()
        ));
    }

    let verify_parent_unchanged = || -> Result<(), String> {
        let cur_meta = std::fs::symlink_metadata(parent_canon).map_err(|e| {
            format!(
                "failed to re-inspect output directory {}: {e}",
                parent_canon.display()
            )
        })?;
        if cur_meta.file_type().is_symlink() || !cur_meta.file_type().is_dir() {
            return Err(format!(
                "output directory {} was replaced by a symlink or non-directory",
                parent_canon.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if cur_meta.dev() != parent_meta.dev() || cur_meta.ino() != parent_meta.ino() {
                return Err(format!(
                    "output directory {} changed during write",
                    parent_canon.display()
                ));
            }
        }
        Ok(())
    };

    verify_parent_unchanged()?;
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

    // Re-verify parent identity and destination immediately before atomic rename.
    if let Err(e) = verify_parent_unchanged() {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
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

fn run() -> Result<(), CliError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        print_usage();
        return Ok(());
    }

    match args[0].as_str() {
        "check" => {
            if args.len() != 2 {
                print_usage();
                return Err("check expects exactly one <pack_dir> argument"
                    .to_owned()
                    .into());
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
        "verify" => {
            if args.len() != 2 {
                print_usage();
                return Err("verify expects exactly one <pack_dir> argument"
                    .to_owned()
                    .into());
            }
            let pack_dir = Path::new(&args[1]);
            let compiled = compile_pack_dir(pack_dir).map_err(|e| e.to_string())?;
            let report = verify_pack_safety(&compiled).map_err(|e| e.to_string())?;
            if report.is_ok() {
                println!(
                    "VERIFIED: {} ({} objects, {} BHAVs, {} lock invariants proven, 0 violations)",
                    report.pack_name,
                    report.objects.len(),
                    report.total_trees_checked(),
                    report.total_locks_verified()
                );
                Ok(())
            } else {
                for obj in &report.objects {
                    for v in &obj.violations {
                        eprintln!(
                            "VIOLATION [{}] {} ({:?} node {:?}): {}",
                            v.object, v.tree_name, v.category, v.node, v.message
                        );
                    }
                }
                Err(format!(
                    "safety verification failed with {} violation(s)",
                    report.total_violations()
                )
                .into())
            }
        }
        "equiv" => {
            let mut antics_path: Option<PathBuf> = None;
            let mut iff_path: Option<PathBuf> = None;
            let mut tree_id: Option<u16> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--tree" | "-t" => {
                        let val = args
                            .get(i + 1)
                            .ok_or_else(|| "missing tree ID after --tree".to_owned())?;
                        tree_id = Some(parse_tree_id(val)?);
                        i += 2;
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag '{other}'").into());
                    }
                    other => {
                        if antics_path.is_none() {
                            antics_path = Some(PathBuf::from(other));
                        } else if iff_path.is_none() {
                            iff_path = Some(PathBuf::from(other));
                        } else {
                            return Err(format!("unexpected extra argument '{other}'").into());
                        }
                        i += 1;
                    }
                }
            }
            let antics_path = antics_path.ok_or_else(|| "equiv requires <a.antics>".to_owned())?;
            let iff_path = iff_path.ok_or_else(|| "equiv requires <b.iff>".to_owned())?;
            let tree_id = tree_id.ok_or_else(|| "equiv requires --tree <id>".to_owned())?;

            let antics_src = std::fs::read_to_string(&antics_path)
                .map_err(|e| format!("failed to read {}: {e}", antics_path.display()))?;
            let compiled = compile_antics(&antics_src).map_err(|e| e.to_string())?;
            let bhav_a = compiled.behaviors.get(&tree_id).ok_or_else(|| {
                format!("{} does not define tree #{tree_id}", antics_path.display())
            })?;
            let iff_bytes = std::fs::read(&iff_path)
                .map_err(|e| format!("failed to read {}: {e}", iff_path.display()))?;
            let bhav_b = extract_bhav_from_iff(&iff_bytes, tree_id).map_err(|e| e.to_string())?;
            let mut arities = extract_bhav_arities_from_iff(&iff_bytes).unwrap_or_default();
            for (&id, bhav) in &compiled.behaviors {
                let argc = match bhav.format {
                    0x8000 => 4,
                    0x8001 => usize::from(bhav.args).min(4),
                    _ => usize::from(bhav.args),
                };
                arities.entry(id).or_insert(argc);
            }

            match check_bhav_equivalent_with_arities(
                bhav_a,
                &bhav_b,
                &EquivOptions::default(),
                Some(&arities),
            ) {
                EquivResult::Equivalent {
                    states_checked,
                    paths_verified,
                } => {
                    println!(
                        "EQUIVALENT: tree #{tree_id} ({states_checked} states checked, {paths_verified} paths verified)"
                    );
                    Ok(())
                }
                EquivResult::Counterexample { path, divergence } => Err(format!(
                    "COUNTEREXAMPLE on tree #{tree_id} after {} step(s): {divergence}",
                    path.len()
                )
                .into()),
                EquivResult::Inconclusive { reason } => {
                    Err(format!("INCONCLUSIVE on tree #{tree_id}: {reason}").into())
                }
            }
        }
        "lift" => cmd_lift(&args),
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
                        return Err(format!("unknown flag '{other}'").into());
                    }
                    other => {
                        if pack_dir.is_some() {
                            return Err(format!("unexpected extra argument '{other}'").into());
                        }
                        pack_dir = Some(PathBuf::from(other));
                        i += 1;
                    }
                }
            }
            let pack_dir = pack_dir.ok_or_else(|| "build requires <pack_dir>".to_owned())?;
            let out_dir = out_dir.ok_or_else(|| "build requires --out <dir>".to_owned())?;

            let surfaces_stat = std::fs::symlink_metadata(pack_dir.join("surfaces.toml"));
            let house_stat = std::fs::symlink_metadata(pack_dir.join("starter-house.toml"));
            let has_world = !matches!(
                (&surfaces_stat, &house_stat),
                (Err(e1), Err(e2))
                    if e1.kind() == std::io::ErrorKind::NotFound
                        && e2.kind() == std::io::ErrorKind::NotFound
            );

            if has_world {
                // Compile and validate world manifests BEFORE creating any output directories.
                let standalone =
                    compile_standalone_pack_dir(&pack_dir).map_err(|e| e.to_string())?;
                let out_canon = ensure_real_dir(&out_dir, None)?;
                let objects_canon = ensure_real_dir(&out_canon.join("objects"), Some(&out_canon))?;
                let sounds_canon = ensure_real_dir(&out_canon.join("sounds"), Some(&out_canon))?;
                let floors_canon = ensure_real_dir(&out_canon.join("floors"), Some(&out_canon))?;
                let walls_canon = ensure_real_dir(&out_canon.join("walls"), Some(&out_canon))?;
                let chars_canon = ensure_real_dir(&out_canon.join("characters"), Some(&out_canon))?;
                let houses_canon = ensure_real_dir(&out_canon.join("houses"), Some(&out_canon))?;
                let nbhd_canon =
                    ensure_real_dir(&out_canon.join("neighborhood"), Some(&out_canon))?;

                write_contained_file(
                    &out_canon,
                    &standalone.pack.archive_name,
                    &standalone.pack.far_bytes,
                )?;
                for entry in &standalone.pack.objects {
                    write_contained_file(&objects_canon, &entry.iff_name, &entry.object.iff_bytes)?;
                }
                for (name, wav) in &standalone.pack.sounds_wav {
                    write_contained_file(&sounds_canon, &format!("{name}.wav"), wav)?;
                }
                for (name, bytes) in &standalone.floors {
                    write_contained_file(&floors_canon, name, bytes)?;
                }
                for (name, bytes) in &standalone.walls {
                    write_contained_file(&walls_canon, name, bytes)?;
                }
                for (name, bytes) in &standalone.characters {
                    write_contained_file(&chars_canon, name, bytes)?;
                }
                write_contained_file(
                    &houses_canon,
                    &standalone.house_iff_name,
                    &standalone.house_iff,
                )?;
                write_contained_file(
                    &nbhd_canon,
                    "Neighborhood.iff",
                    &standalone.neighborhood_iff,
                )?;
                println!(
                    "Built standalone {} v{} -> {} ({} objects, {} floors, {} walls, {} chars, {}, {} sims, {} bytes FAR, {} WAVs)",
                    standalone.pack.name,
                    standalone.pack.version,
                    out_dir.display(),
                    standalone.pack.objects.len(),
                    standalone.floors.len(),
                    standalone.walls.len(),
                    standalone.characters.len(),
                    standalone.house_iff_name,
                    standalone.starter_sims.len(),
                    standalone.pack.far_bytes.len(),
                    standalone.pack.sounds_wav.len()
                );
                Ok(())
            } else {
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
        }
        other => {
            print_usage();
            Err(format!("unknown subcommand '{other}'").into())
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) | Err(CliError::BrokenPipe) => ExitCode::SUCCESS,
        Err(CliError::Message(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
