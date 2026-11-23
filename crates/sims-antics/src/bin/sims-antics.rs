//! CLI entrypoint for expanding `.antics` v2 sources to `.air` (Antics IR),
//! compiling packs/objects, verifying safety, checking equivalence, and inspecting
//! lifted `.air` behavior trees from `.iff` files and `.far` archives.
//!
//! Usage:
//! - `sims-antics expand <file.antics> [--pack <pack_dir>]`
//! - `sims-antics check <pack_dir|file.antics> [--pack <pack_dir>]`
//! - `sims-antics verify <pack_dir|file.antics> [--pack <pack_dir>]`
//! - `sims-antics equiv <a.antics|a.air> <b.iff> --tree <id> [--pack <pack_dir>]`
//! - `sims-antics verify-equiv <a.iff> <b.iff> [--tree <id>] [--with <other.iff>]... [--my-attributes <n>] [--ignore-scratch-pointer] [--strict-frame-residue] [--all]`
//!   (exit 0 equivalent, 1 different, 2 inconclusive or error)
//! - `sims-antics lift <file.iff|archive.far> [--entry <name.iff>] [--tree <id>] [--list] [--filter <substr>] [--raw] [--relax-padding] [--no-comments] [--verify-equiv]`
//! - `sims-antics build <pack_dir> --out <dir>`

use sims_antics::{
    EquivOptions, EquivResult, IffEquivOptions, IffVerdict, LiftOptions, LiveContext, LiveOptions,
    ResourceVerdict, StructuredLiftOutcome, check_bhav_equivalent_combined,
    check_bhav_equivalent_with_arities, check_iff_equivalent, compile_air, compile_pack_dir,
    compile_standalone_pack_dir, compile_v2_object_file, expand_v2_to_air,
    extract_lift_symbols_from_iff,
    lift::{format_bhav_labeled_air_with_context, format_bhav_raw_air, lift_bhav},
    lift_bhav_structured_with_arities, lift_bhav_structured_with_context,
    v2::is_v2_source,
    verify::{extract_bhav_arities_from_iff, extract_bhav_from_iff},
    verify_object_safety, verify_pack_safety,
};
use sims_formats::far;
use sims_formats::iff::{DELETED, Iff};
use std::ffi::{OsStr, OsString};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
enum CliError {
    Message(String),
    BrokenPipe,
    /// Exit with this status after the subcommand printed its own report.
    Exit(u8),
}

impl From<String> for CliError {
    fn from(msg: String) -> Self {
        Self::Message(msg)
    }
}

struct OutWriter<W: Write> {
    writer: W,
}

impl OutWriter<BufWriter<std::io::StdoutLock<'static>>> {
    fn stdout() -> Self {
        Self {
            writer: BufWriter::new(std::io::stdout().lock()),
        }
    }
}

impl<W: Write> OutWriter<W> {
    #[cfg(test)]
    fn from_writer(writer: W) -> Self {
        Self { writer }
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

#[derive(Default)]
struct LiftSummary {
    counterexamples: usize,
    compile_errors: usize,
    inconclusive: usize,
    parse_errors: usize,
}

impl LiftSummary {
    fn into_result(self, broken_pipe: bool) -> Result<(), CliError> {
        if self.counterexamples > 0 || self.compile_errors > 0 {
            let mut msg = format!(
                "verify-equiv failed ({} counterexample(s), {} compile error(s)",
                self.counterexamples, self.compile_errors
            );
            if self.inconclusive > 0 {
                msg.push_str(&format!(", {} inconclusive", self.inconclusive));
            }
            msg.push(')');
            if self.parse_errors > 0 {
                msg.push_str(&format!("; {} BHAV(s) failed to parse", self.parse_errors));
            }
            return Err(CliError::Message(msg));
        }
        if self.parse_errors > 0 {
            return Err(CliError::Message(format!(
                "failed to parse {} BHAV chunk(s)",
                self.parse_errors
            )));
        }
        if self.inconclusive > 0 {
            eprintln!(
                "warning: verify-equiv reported {} inconclusive tree(s)",
                self.inconclusive
            );
        }
        if broken_pipe {
            return Err(CliError::BrokenPipe);
        }
        Ok(())
    }
}

fn os_arg_to_str<'a>(arg: &'a OsStr, context: &str) -> Result<&'a str, CliError> {
    arg.to_str()
        .ok_or_else(|| CliError::Message(format!("{context} contains invalid UTF-8")))
}

fn os_arg_starts_with_dash(arg: &OsStr) -> bool {
    arg.as_encoded_bytes().starts_with(b"-")
}

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         sims-antics expand <file.antics> [--pack <pack_dir>]\n  \
         sims-antics check <pack_dir|file.antics> [--pack <pack_dir>]\n  \
         sims-antics verify <pack_dir|file.antics> [--pack <pack_dir>]\n  \
         sims-antics equiv <a.antics|a.air> <b.iff> --tree <id> [--pack <pack_dir>]\n  \
         sims-antics verify-equiv <a.iff> <b.iff> [--tree <id>] [--with <other.iff>]... [--my-attributes <n>] [--ignore-scratch-pointer] [--strict-frame-residue] [--all]   (exit 0 equivalent, 1 different, 2 inconclusive)\n  \
         sims-antics lift <file.iff|archive.far> [--entry <name.iff>] [--tree <id>] [--list] [--filter <substr>] [--raw] [--relax-padding] [--no-comments] [--verify-equiv]   (lifts to .air Antics IR)\n  \
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
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "\\x{b:02x}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn sanitize_path_display(path: &Path) -> String {
    sanitize_single_line(&path.display().to_string())
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
        "ambiguous --entry '{}' matched {} FAR entries:",
        sanitize_single_line(query),
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

    // 2. Unicode case-insensitive exact match (normalizing `\` and `/`).
    let q_norm = normalize_archive_path(query);
    let q_folded = q_norm.to_lowercase();
    let ci_exact: Vec<&far::Entry> = entries
        .iter()
        .filter(|e| normalize_archive_path(&e.name).to_lowercase() == q_folded)
        .collect();
    if ci_exact.len() == 1 {
        return Ok(ci_exact[0]);
    }
    if ci_exact.len() > 1 {
        return Err(format_ambiguous_matches(query, &ci_exact));
    }

    // 3. Unique path-segment suffix / basename match (Unicode case-insensitive, separator-normalized).
    let q_trimmed = q_norm.trim_start_matches('/');
    if !q_trimmed.is_empty() {
        let q_lower = q_trimmed.to_lowercase();
        let seg_suffix: Vec<&far::Entry> = entries
            .iter()
            .filter(|e| {
                let norm = normalize_archive_path(&e.name).to_lowercase();
                if norm == q_lower {
                    true
                } else if let Some(prefix) = norm.strip_suffix(&q_lower) {
                    prefix.ends_with('/')
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

        // 4. Unique general Unicode case-insensitive suffix fallback.
        let general_suffix: Vec<&far::Entry> = entries
            .iter()
            .filter(|e| {
                normalize_archive_path(&e.name)
                    .to_lowercase()
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
        "FAR entry '{}' not found in {} (use --list to list .iff entries)",
        sanitize_single_line(query),
        sanitize_path_display(archive_path)
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
    if label.to_lowercase().contains(filter_lower) {
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

fn open_regular_file_nonblocking(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NONBLOCK: i32 = 0x0004;
        opts.custom_flags(O_NONBLOCK);
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NONBLOCK: i32 = 0x0800;
        opts.custom_flags(O_NONBLOCK);
    }
    opts.open(path)
}

fn read_regular_file_prefix(path: &Path) -> Result<(std::fs::File, u64, Vec<u8>), String> {
    let safe_path = sanitize_path_display(path);
    // Reject non-regular files (such as FIFOs/directories) before opening so a named pipe
    // cannot block waiting for a writer, then open non-blocking and re-verify the opened fd.
    let pre_meta =
        std::fs::metadata(path).map_err(|e| format!("failed to stat {safe_path}: {e}"))?;
    if !pre_meta.file_type().is_file() {
        return Err(format!("{safe_path} is not a regular file"));
    }
    let mut file = open_regular_file_nonblocking(path)
        .map_err(|e| format!("failed to open {safe_path}: {e}"))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("failed to stat {safe_path}: {e}"))?;
    if !meta.file_type().is_file() {
        return Err(format!("{safe_path} is not a regular file"));
    }
    let file_len = meta.len();
    let mut header = Vec::with_capacity(16);
    (&mut file)
        .take(16)
        .read_to_end(&mut header)
        .map_err(|e| format!("failed to read {safe_path}: {e}"))?;
    Ok((file, file_len, header))
}

fn read_bounded_iff_file(
    path: &Path,
    mut file: std::fs::File,
    file_len: u64,
    header: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let safe_path = sanitize_path_display(path);
    if file_len > far::MAX_MEMBER {
        return Err(format!(
            "{safe_path} exceeds maximum IFF size ({file_len} bytes > {} bytes)",
            far::MAX_MEMBER
        ));
    }
    let mut bytes = Vec::with_capacity(file_len as usize);
    bytes.extend_from_slice(&header);
    let max_rest = far::MAX_MEMBER.saturating_sub(header.len() as u64);
    (&mut file)
        .take(max_rest.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read {safe_path}: {e}"))?;
    if bytes.len() as u64 > far::MAX_MEMBER {
        return Err(format!(
            "{safe_path} grew past maximum IFF size ({} bytes)",
            far::MAX_MEMBER
        ));
    }
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn emit_lifted_bhavs<W: Write>(
    iff: &Iff,
    selected_chunks: &[&sims_formats::iff::Chunk],
    source_label: &str,
    tree_id: Option<u16>,
    raw: bool,
    lift_opts: LiftOptions,
    verify_equiv: bool,
    out: &mut OutWriter<W>,
) -> Result<(), CliError> {
    if selected_chunks.is_empty() {
        out.writeln_str(&format!(
            "// no BHAV chunks found in {}",
            sanitize_single_line(source_label)
        ))?;
        out.flush()?;
        return Ok(());
    }

    // Match `sims_vm::interpreter::behavior_from_owner`: in a standalone IFF without an
    // install catalog, only private/local BHAVs (`4096..=8191`) are guaranteed to resolve
    // within this file; global (`256..=4095`) and semi-global (`8192..=65535`) callees
    // must remain unknown so stray chunks in an object IFF cannot mask external arities.
    let mut local_arities = extract_bhav_arities_from_iff(&iff.bytes).unwrap_or_default();
    local_arities.retain(|&id, _| (4096..=8191).contains(&id));
    let arities = Some(local_arities);
    let symbols = if lift_opts.annotate_comments {
        Some(extract_lift_symbols_from_iff(iff))
    } else {
        None
    };

    let mut summary = LiftSummary::default();
    let mut broken_pipe = false;

    macro_rules! write_or_break {
        ($expr:expr) => {
            match $expr {
                Ok(()) => {}
                Err(CliError::BrokenPipe) => {
                    broken_pipe = true;
                    break;
                }
                Err(other) => return Err(other),
            }
        };
    }

    for (idx, chunk) in selected_chunks.iter().enumerate() {
        if idx > 0 {
            write_or_break!(out.writeln_str(""));
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
                summary.parse_errors += 1;
                write_or_break!(out.writeln_str(&format!(
                    "// tree {} \"{}\": failed to parse BHAV ({})",
                    chunk.id,
                    sanitize_single_line(&chunk.label),
                    sanitize_single_line(&err.to_string())
                )));
                continue;
            }
        };

        if raw {
            write_or_break!(out.writeln_str(&format!(
                "// .air (Antics IR) raw tree {} \"{}\"",
                bhav.id,
                sanitize_single_line(&bhav.label)
            )));
            let src = format_bhav_raw_air(&bhav);
            write_or_break!(out.write_str(&src));
            if !src.ends_with('\n') {
                write_or_break!(out.writeln_str(""));
            }
            continue;
        }

        let outcome =
            lift_bhav_structured_with_context(&bhav, lift_opts, arities.as_ref(), symbols.as_ref());
        let emitted_src = match outcome {
            StructuredLiftOutcome::Lifted(src) => {
                write_or_break!(out.writeln_str(&format!(
                    "// .air (Antics IR) tree {} \"{}\"",
                    bhav.id,
                    sanitize_single_line(&bhav.label)
                )));
                write_or_break!(out.write_str(&src));
                if !src.ends_with('\n') {
                    write_or_break!(out.writeln_str(""));
                }
                src
            }
            other => {
                let reason = lift_reason(&other).unwrap_or_else(|| "unknown".to_owned());
                write_or_break!(out.writeln_str(&format!(
                    "// tree {} \"{}\": structured lift fallback ({reason}); showing decoded labeled-CFG .air (Antics IR)",
                    bhav.id,
                    sanitize_single_line(&bhav.label)
                )));
                let labeled_src = format_bhav_labeled_air_with_context(
                    &bhav,
                    lift_opts,
                    arities.as_ref(),
                    symbols.as_ref(),
                );
                write_or_break!(out.write_str(&labeled_src));
                if !labeled_src.ends_with('\n') {
                    write_or_break!(out.writeln_str(""));
                }
                labeled_src
            }
        };
        if verify_equiv {
            let status_line = match compile_air(&emitted_src) {
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
                            } => format!(
                                "// verify-equiv: Equivalent ({states_checked} states checked, {paths_verified} paths verified)"
                            ),
                            EquivResult::Counterexample { path, divergence } => {
                                summary.counterexamples += 1;
                                format!(
                                    "// verify-equiv: Counterexample(after {} step(s): {})",
                                    path.len(),
                                    sanitize_single_line(&divergence)
                                )
                            }
                            EquivResult::Inconclusive { reason } => {
                                summary.inconclusive += 1;
                                format!(
                                    "// verify-equiv: Inconclusive({})",
                                    sanitize_single_line(&reason)
                                )
                            }
                        }
                    } else {
                        summary.compile_errors += 1;
                        format!(
                            "// verify-equiv: CompileError(recompiled output missing tree #{})",
                            bhav.id
                        )
                    }
                }
                Err(err) => {
                    summary.compile_errors += 1;
                    format!(
                        "// verify-equiv: CompileError({})",
                        sanitize_single_line(&err.to_string())
                    )
                }
            };
            write_or_break!(out.writeln_str(&status_line));
        }
    }

    if !broken_pipe {
        match out.flush() {
            Ok(()) => {}
            Err(CliError::BrokenPipe) => broken_pipe = true,
            Err(other) => return Err(other),
        }
    }

    summary.into_result(broken_pipe)
}

fn cmd_lift(args: &[OsString]) -> Result<(), CliError> {
    let mut input_path: Option<PathBuf> = None;
    let mut entry_name: Option<String> = None;
    let mut tree_id: Option<u16> = None;
    let mut list = false;
    let mut filter: Option<String> = None;
    let mut raw = false;
    let mut relax_padding = false;
    let mut no_comments = false;
    let mut verify_equiv = false;
    let mut end_of_flags = false;

    let mut i = 1usize;
    while i < args.len() {
        let arg = args[i].as_os_str();
        if !end_of_flags {
            if let Some(s) = arg.to_str() {
                match s {
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
                        entry_name = Some(os_arg_to_str(val, "--entry value")?.to_owned());
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
                        tree_id = Some(parse_tree_id(os_arg_to_str(val, "--tree value")?)?);
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
                        filter = Some(os_arg_to_str(val, "--filter value")?.to_owned());
                        i += 2;
                        continue;
                    }
                    "--raw" => {
                        raw = true;
                        i += 1;
                        continue;
                    }
                    "--relax-padding" => {
                        relax_padding = true;
                        i += 1;
                        continue;
                    }
                    "--no-comments" => {
                        no_comments = true;
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
            } else if os_arg_starts_with_dash(arg) {
                return Err(
                    "unknown non-UTF-8 flag (use '--' before file paths starting with '-')"
                        .to_owned()
                        .into(),
                );
            }
        }
        if input_path.is_some() {
            return Err(format!("unexpected extra argument '{}'", Path::new(arg).display()).into());
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
    if raw && relax_padding {
        return Err("--relax-padding cannot be combined with --raw"
            .to_owned()
            .into());
    }
    if tree_id.is_some() && filter.is_some() {
        return Err("--filter cannot be combined with --tree".to_owned().into());
    }
    let lift_opts = LiftOptions {
        strict_padding: !relax_padding,
        annotate_comments: !no_comments,
    };

    let safe_input_path = sanitize_path_display(&input_path);
    let (mut file, file_len, header) = read_regular_file_prefix(&input_path)?;
    let has_far_ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("far"));
    let has_far_magic = header.len() >= 8 && &header[..8] == b"FAR!byAZ";
    let is_far = has_far_ext || has_far_magic;

    let mut out = OutWriter::stdout();

    let (iff_bytes, source_label) = if is_far {
        if header.len() < 16 {
            return Err(format!(
                "invalid FAR archive {safe_input_path}: file shorter than 16-byte header"
            )
            .into());
        }
        let manifest_off = far::manifest_offset(&header, file_len)
            .map_err(|e| format!("failed to parse FAR header in {safe_input_path}: {e}"))?;
        let dir_len = far::directory_len(manifest_off, file_len)
            .map_err(|e| format!("invalid FAR directory in {safe_input_path}: {e}"))?;
        file.seek(SeekFrom::Start(u64::from(manifest_off)))
            .map_err(|e| format!("failed to seek {safe_input_path}: {e}"))?;
        let mut dir_buf = vec![0u8; dir_len];
        file.read_exact(&mut dir_buf)
            .map_err(|e| format!("failed to read FAR directory in {safe_input_path}: {e}"))?;
        let entries = far::directory(&dir_buf, file_len)
            .map_err(|e| format!("failed to parse FAR directory in {safe_input_path}: {e}"))?;

        match entry_name {
            None => {
                if !list {
                    return Err(format!(
                        "{safe_input_path} is a FAR archive; pass --list to list .iff entries or --entry <name.iff> to inspect one"
                    )
                    .into());
                }
                let filter_lower = filter.as_deref().map(str::to_lowercase);
                let mut iff_entries: Vec<&far::Entry> = entries
                    .iter()
                    .filter(|e| is_iff_entry_name(&e.name))
                    .filter(|e| match &filter_lower {
                        Some(needle) => e.name.to_lowercase().contains(needle),
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
                let safe_entry = sanitize_single_line(&entry.name);
                if !entry.within_payload(manifest_off) {
                    return Err(format!(
                        "FAR entry '{safe_entry}' in {safe_input_path} has out-of-bounds payload"
                    )
                    .into());
                }
                if entry.stored_size != entry.unpacked_size {
                    return Err(format!(
                        "compressed FAR entry '{safe_entry}' ({} stored / {} unpacked) is not supported",
                        entry.stored_size, entry.unpacked_size
                    )
                    .into());
                }
                if u64::from(entry.stored_size) > far::MAX_MEMBER {
                    return Err(format!(
                        "FAR entry '{safe_entry}' exceeds maximum member size ({} > {})",
                        entry.stored_size,
                        far::MAX_MEMBER
                    )
                    .into());
                }
                file.seek(SeekFrom::Start(u64::from(entry.offset)))
                    .map_err(|e| format!("failed to seek {safe_input_path}: {e}"))?;
                let mut member_bytes = vec![0u8; entry.stored_size as usize];
                file.read_exact(&mut member_bytes).map_err(|e| {
                    format!("failed to read FAR entry '{safe_entry}' from {safe_input_path}: {e}")
                })?;
                let label = format!("{safe_input_path}!{safe_entry}");
                (member_bytes, label)
            }
        }
    } else {
        if entry_name.is_some() {
            return Err(format!(
                "--entry is only valid for .far archives ({safe_input_path} is not a FAR archive)"
            )
            .into());
        }
        let bytes = read_bounded_iff_file(&input_path, file, file_len, header)?;
        (bytes, safe_input_path)
    };

    let iff =
        Iff::parse(iff_bytes).map_err(|e| format!("failed to parse IFF {source_label}: {e}"))?;

    // Deduplicate BHAV chunks by resource ID, preferring canonical high-flag-byte 0
    // (matching `Iff::get(b"BHAV", id)` and `sims-vm`), then sort by ID.
    let mut by_id: std::collections::BTreeMap<u16, &sims_formats::iff::Chunk> =
        std::collections::BTreeMap::new();
    for chunk in iff.all(b"BHAV").filter(|c| c.flags & DELETED == 0) {
        by_id
            .entry(chunk.id)
            .and_modify(|existing| {
                if (existing.flags >> 8) != 0 && (chunk.flags >> 8) == 0 {
                    *existing = chunk;
                }
            })
            .or_insert(chunk);
    }
    let bhav_chunks: Vec<&sims_formats::iff::Chunk> = by_id.into_values().collect();

    if list {
        let mut local_arities = extract_bhav_arities_from_iff(&iff.bytes).unwrap_or_default();
        local_arities.retain(|&id, _| (4096..=8191).contains(&id));
        let filter_lower = filter.as_deref().map(str::to_lowercase);
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
                    let list_opts = LiftOptions {
                        strict_padding: lift_opts.strict_padding,
                        annotate_comments: false,
                    };
                    let status = match lift_bhav_structured_with_arities(
                        &bhav,
                        list_opts,
                        Some(&local_arities),
                    ) {
                        StructuredLiftOutcome::Lifted(src) => {
                            let raw_count = src.matches("op 0x").count();
                            if raw_count == 0 {
                                "structured".to_owned()
                            } else {
                                format!("structured ({raw_count} inline raw op)")
                            }
                        }
                        other => format!(
                            "labeled-cfg ({})",
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
        let needle = needle_raw.to_lowercase();
        let matched: Vec<_> = bhav_chunks
            .into_iter()
            .filter(|c| bhav_matches_filter(c.id, &c.label, &needle))
            .collect();
        if matched.is_empty() {
            return Err(format!(
                "no BHAV chunks matched filter '{}' in {source_label}",
                sanitize_single_line(needle_raw)
            )
            .into());
        }
        matched
    } else {
        bhav_chunks
    };

    emit_lifted_bhavs(
        &iff,
        &selected_chunks,
        &source_label,
        tree_id,
        raw,
        lift_opts,
        verify_equiv,
        &mut out,
    )
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

fn parse_path_and_optional_pack(
    args: &[OsString],
    cmd_name: &str,
) -> Result<(PathBuf, Option<PathBuf>), CliError> {
    let mut target_path: Option<PathBuf> = None;
    let mut pack_dir: Option<PathBuf> = None;
    let mut end_of_flags = false;
    let mut i = 1usize;
    while i < args.len() {
        let arg = args[i].as_os_str();
        if !end_of_flags {
            if let Some(s) = arg.to_str() {
                match s {
                    "-h" | "--help" => {
                        print_usage();
                        return Err(CliError::BrokenPipe);
                    }
                    "--" => {
                        end_of_flags = true;
                        i += 1;
                        continue;
                    }
                    "--pack" | "-p" => {
                        if pack_dir.is_some() {
                            return Err("duplicate --pack flag".to_owned().into());
                        }
                        let val = args
                            .get(i + 1)
                            .ok_or_else(|| "missing directory path after --pack".to_owned())?;
                        pack_dir = Some(PathBuf::from(val));
                        i += 2;
                        continue;
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag '{other}'").into());
                    }
                    _ => {}
                }
            } else if os_arg_starts_with_dash(arg) {
                return Err(
                    "unknown non-UTF-8 flag (use '--' before file paths starting with '-')"
                        .to_owned()
                        .into(),
                );
            }
        }
        if target_path.is_some() {
            return Err(format!("unexpected extra argument '{}'", Path::new(arg).display()).into());
        }
        target_path = Some(PathBuf::from(arg));
        i += 1;
    }
    let target_path = target_path.ok_or_else(|| {
        print_usage();
        format!("{cmd_name} requires a path argument")
    })?;
    Ok((target_path, pack_dir))
}

fn cmd_expand(args: &[OsString]) -> Result<(), CliError> {
    let (file_path, pack_dir) = match parse_path_and_optional_pack(args, "expand") {
        Ok(pair) => pair,
        Err(CliError::BrokenPipe) => return Ok(()),
        Err(e) => return Err(e),
    };
    let air = expand_v2_to_air(&file_path, pack_dir.as_deref()).map_err(|e| e.to_string())?;
    let mut out = OutWriter::stdout();
    out.write_str(&air)?;
    if !air.ends_with('\n') {
        out.writeln_str("")?;
    }
    out.flush()?;
    Ok(())
}

/// Reads a regular UTF-8 text file safely without following symlinks or blocking on FIFOs.
fn read_regular_text_file(path: &Path) -> Result<String, CliError> {
    let safe_path = sanitize_path_display(path);
    let pre_meta =
        std::fs::symlink_metadata(path).map_err(|e| format!("failed to stat {safe_path}: {e}"))?;
    if pre_meta.file_type().is_symlink() || !pre_meta.file_type().is_file() {
        return Err(format!(
            "{safe_path} is not a regular file (refusing symlinks and special files)"
        )
        .into());
    }
    let mut file = open_regular_file_nonblocking(path)
        .map_err(|e| format!("failed to open {safe_path}: {e}"))?;
    let open_meta = file
        .metadata()
        .map_err(|e| format!("failed to stat {safe_path}: {e}"))?;
    if !open_meta.file_type().is_file() {
        return Err(format!("{safe_path} is not a regular file").into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if open_meta.dev() != pre_meta.dev() || open_meta.ino() != pre_meta.ino() {
            return Err(format!("{safe_path} changed between validation and open").into());
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("failed to read {safe_path}: {e}"))?;
    Ok(text)
}

fn write_stdout_line(line: &str) -> Result<(), CliError> {
    let mut out = OutWriter::stdout();
    out.writeln_str(line)?;
    out.flush()
}

fn run() -> Result<(), CliError> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        print_usage();
        return Ok(());
    }
    let subcmd = os_arg_to_str(&args[0], "subcommand")?;
    if matches!(subcmd, "-h" | "--help" | "help") {
        print_usage();
        return Ok(());
    }

    match subcmd {
        "expand" => cmd_expand(&args),
        "check" => {
            let (target, pack_dir) = match parse_path_and_optional_pack(&args, "check") {
                Ok(pair) => pair,
                Err(CliError::BrokenPipe) => return Ok(()),
                Err(e) => return Err(e),
            };
            let meta = std::fs::symlink_metadata(&target)
                .map_err(|e| format!("failed to access {}: {e}", target.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("refusing symlink target {}", target.display()).into());
            }
            if meta.file_type().is_dir() {
                if pack_dir.is_some() {
                    return Err("--pack is only valid when checking a single .antics file"
                        .to_owned()
                        .into());
                }
                let compiled = compile_pack_dir(&target).map_err(|e| e.to_string())?;
                let total_bhavs: usize = compiled
                    .objects
                    .iter()
                    .map(|o| o.object.behaviors.len())
                    .sum();
                return write_stdout_line(&format!(
                    "OK: {} v{} ({} objects, {} BHAVs, {} bytes {}, {} WAVs)",
                    compiled.name,
                    compiled.version,
                    compiled.objects.len(),
                    total_bhavs,
                    compiled.far_bytes.len(),
                    compiled.archive_name,
                    compiled.sounds_wav.len()
                ));
            }
            if !meta.file_type().is_file() {
                return Err(
                    format!("{} is not a regular file or directory", target.display()).into(),
                );
            }
            // Validate `--pack` containment before reading source bytes, then dispatch by format.
            let src = if let Some(ref pdir) = pack_dir {
                sims_antics::v2::loader::read_v2_checked_file(&target, Some(pdir))
                    .map_err(|e| e.to_string())?
                    .0
            } else {
                read_regular_text_file(&target)?
            };
            if is_v2_source(&src) {
                let obj = compile_v2_object_file(&target, pack_dir.as_deref())
                    .map_err(|e| e.to_string())?;
                write_stdout_line(&format!(
                    "OK: {} (GUID {:#010x}, {} BHAVs, {} bytes IFF, {} WAVs)",
                    obj.name,
                    obj.guid,
                    obj.behaviors.len(),
                    obj.iff_bytes.len(),
                    obj.sounds_wav.len()
                ))
            } else {
                let module = compile_air(&src).map_err(|e| e.to_string())?;
                write_stdout_line(&format!(
                    "OK: .air module {} ({} BHAVs, {} BCONs, {} STR#s)",
                    sanitize_path_display(&target),
                    module.behaviors.len(),
                    module.constants.len(),
                    module.strings.len()
                ))
            }
        }
        "verify" => {
            let (target, pack_dir) = match parse_path_and_optional_pack(&args, "verify") {
                Ok(pair) => pair,
                Err(CliError::BrokenPipe) => return Ok(()),
                Err(e) => return Err(e),
            };
            let meta = std::fs::symlink_metadata(&target)
                .map_err(|e| format!("failed to access {}: {e}", target.display()))?;
            if meta.file_type().is_symlink() {
                return Err(format!("refusing symlink target {}", target.display()).into());
            }
            if meta.file_type().is_file() {
                let obj = compile_v2_object_file(&target, pack_dir.as_deref())
                    .map_err(|e| e.to_string())?;
                let report = verify_object_safety(&obj).map_err(|e| e.to_string())?;
                if report.is_ok() {
                    return write_stdout_line(&format!(
                        "VERIFIED: {} ({} BHAVs, {} lock invariants proven, 0 violations)",
                        obj.name, report.trees_checked, report.locks_verified
                    ));
                }
                for v in &report.violations {
                    eprintln!(
                        "VIOLATION [{}] {} ({:?} node {:?}): {}",
                        v.object, v.tree_name, v.category, v.node, v.message
                    );
                }
                return Err(format!(
                    "safety verification failed with {} violation(s)",
                    report.violations.len()
                )
                .into());
            }
            if !meta.file_type().is_dir() {
                return Err(
                    format!("{} is not a regular file or directory", target.display()).into(),
                );
            }
            if pack_dir.is_some() {
                return Err("--pack is only valid when verifying a single .antics file"
                    .to_owned()
                    .into());
            }
            let compiled = compile_pack_dir(&target).map_err(|e| e.to_string())?;
            let report = verify_pack_safety(&compiled).map_err(|e| e.to_string())?;
            if report.is_ok() {
                write_stdout_line(&format!(
                    "VERIFIED: {} ({} objects, {} BHAVs, {} lock invariants proven, 0 violations)",
                    report.pack_name,
                    report.objects.len(),
                    report.total_trees_checked(),
                    report.total_locks_verified()
                ))
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
            let mut pack_dir: Option<PathBuf> = None;
            let mut i = 1usize;
            while i < args.len() {
                let arg = args[i].as_os_str();
                if let Some(s) = arg.to_str() {
                    match s {
                        "--tree" | "-t" => {
                            let val = args
                                .get(i + 1)
                                .ok_or_else(|| "missing tree ID after --tree".to_owned())?;
                            tree_id = Some(parse_tree_id(os_arg_to_str(val, "--tree value")?)?);
                            i += 2;
                            continue;
                        }
                        "--pack" | "-p" => {
                            let val = args
                                .get(i + 1)
                                .ok_or_else(|| "missing directory path after --pack".to_owned())?;
                            pack_dir = Some(PathBuf::from(val));
                            i += 2;
                            continue;
                        }
                        other if other.starts_with('-') => {
                            return Err(format!("unknown flag '{other}'").into());
                        }
                        _ => {}
                    }
                } else if os_arg_starts_with_dash(arg) {
                    return Err("unknown non-UTF-8 flag".to_owned().into());
                }
                if antics_path.is_none() {
                    antics_path = Some(PathBuf::from(arg));
                } else if iff_path.is_none() {
                    iff_path = Some(PathBuf::from(arg));
                } else {
                    return Err(format!(
                        "unexpected extra argument '{}'",
                        Path::new(arg).display()
                    )
                    .into());
                }
                i += 1;
            }
            let antics_path = antics_path.ok_or_else(|| "equiv requires <a.antics>".to_owned())?;
            let iff_path = iff_path.ok_or_else(|| "equiv requires <b.iff>".to_owned())?;
            let tree_id = tree_id.ok_or_else(|| "equiv requires --tree <id>".to_owned())?;

            let antics_src = if let Some(ref pdir) = pack_dir {
                sims_antics::v2::loader::read_v2_checked_file(&antics_path, Some(pdir))
                    .map_err(|e| e.to_string())?
                    .0
            } else {
                read_regular_text_file(&antics_path)?
            };
            let behaviors = if is_v2_source(&antics_src) {
                compile_v2_object_file(&antics_path, pack_dir.as_deref())
                    .map_err(|e| e.to_string())?
                    .behaviors
            } else {
                compile_air(&antics_src)
                    .map_err(|e| e.to_string())?
                    .behaviors
            };
            let bhav_a = behaviors.get(&tree_id).ok_or_else(|| {
                format!("{} does not define tree #{tree_id}", antics_path.display())
            })?;
            let (iff_file, iff_len, iff_hdr) = read_regular_file_prefix(&iff_path)?;
            let iff_bytes = read_bounded_iff_file(&iff_path, iff_file, iff_len, iff_hdr)?;
            let bhav_b = extract_bhav_from_iff(&iff_bytes, tree_id).map_err(|e| e.to_string())?;
            let mut arities = extract_bhav_arities_from_iff(&iff_bytes).unwrap_or_default();
            for (&id, bhav) in &behaviors {
                let argc = match bhav.format {
                    0x8000 => 4,
                    0x8001 => usize::from(bhav.args).min(4),
                    _ => usize::from(bhav.args),
                };
                arities.entry(id).or_insert(argc);
            }

            let ctx = LiveContext {
                callee_arities: Some(arities),
                ..LiveContext::default()
            };
            let (method, result) = check_bhav_equivalent_combined(
                bhav_a,
                &ctx,
                &bhav_b,
                &ctx,
                &EquivOptions::default(),
                &LiveOptions::default(),
            );
            match result {
                EquivResult::Equivalent {
                    states_checked,
                    paths_verified,
                } => write_stdout_line(&format!(
                    "EQUIVALENT: tree #{tree_id} ({method}: {states_checked} states checked, {paths_verified} paths verified)"
                )),
                EquivResult::Counterexample { path, divergence } => Err(format!(
                    "COUNTEREXAMPLE on tree #{tree_id} ({method}) after {} step(s): {divergence}",
                    path.len()
                )
                .into()),
                EquivResult::Inconclusive { reason } => {
                    Err(format!("INCONCLUSIVE on tree #{tree_id}: {reason}").into())
                }
            }
        }
        "lift" => cmd_lift(&args),
        "verify-equiv" => match cmd_verify_equiv(&args) {
            Err(CliError::Message(msg)) => {
                eprintln!("error: {msg}");
                Err(CliError::Exit(2))
            }
            other => other,
        },
        "build" => {
            let mut pack_dir: Option<PathBuf> = None;
            let mut out_dir: Option<PathBuf> = None;
            let mut i = 1usize;
            while i < args.len() {
                let arg = args[i].as_os_str();
                if let Some(s) = arg.to_str() {
                    match s {
                        "--out" | "-o" => {
                            let val = args
                                .get(i + 1)
                                .ok_or_else(|| "missing directory path after --out".to_owned())?;
                            out_dir = Some(PathBuf::from(val));
                            i += 2;
                            continue;
                        }
                        other if other.starts_with('-') => {
                            return Err(format!("unknown flag '{other}'").into());
                        }
                        _ => {}
                    }
                } else if os_arg_starts_with_dash(arg) {
                    return Err("unknown non-UTF-8 flag".to_owned().into());
                }
                if pack_dir.is_some() {
                    return Err(format!(
                        "unexpected extra argument '{}'",
                        Path::new(arg).display()
                    )
                    .into());
                }
                pack_dir = Some(PathBuf::from(arg));
                i += 1;
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

/// `sims-antics verify-equiv <a.iff> <b.iff>`: whole-file resource and BHAV equivalence.
fn cmd_verify_equiv(args: &[OsString]) -> Result<(), CliError> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut with: Vec<PathBuf> = Vec::new();
    let mut opts = IffEquivOptions::default();
    let mut show_all = false;
    let mut i = 1usize;
    while i < args.len() {
        let arg = args[i].as_os_str();
        if let Some(s) = arg.to_str() {
            let value = |name: &str| -> Result<&OsString, CliError> {
                args.get(i + 1)
                    .ok_or_else(|| CliError::Message(format!("missing value after {name}")))
            };
            match s {
                "--tree" | "-t" => {
                    opts.tree = Some(parse_tree_id(os_arg_to_str(value(s)?, "--tree value")?)?);
                    i += 2;
                    continue;
                }
                "--with" => {
                    with.push(PathBuf::from(value(s)?));
                    i += 2;
                    continue;
                }
                "--my-attributes" => {
                    let v = os_arg_to_str(value(s)?, "--my-attributes value")?;
                    let n: usize = v
                        .parse()
                        .map_err(|_| format!("invalid --my-attributes count '{v}'"))?;
                    if n < sims_antics::GUARANTEED_ATTRIBUTES {
                        return Err(format!(
                            "--my-attributes must be at least {} (every object has that many)",
                            sims_antics::GUARANTEED_ATTRIBUTES
                        )
                        .into());
                    }
                    opts.my_attribute_count = n;
                    i += 2;
                    continue;
                }
                "--ignore-scratch-pointer" => {
                    opts.live.observe_scratch_pointer = false;
                    i += 1;
                    continue;
                }
                "--strict-frame-residue" => {
                    opts.live.observe_frame_residue = true;
                    i += 1;
                    continue;
                }
                "--all" => {
                    show_all = true;
                    i += 1;
                    continue;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag '{other}'").into());
                }
                _ => {}
            }
        } else if os_arg_starts_with_dash(arg) {
            return Err("unknown non-UTF-8 flag".to_owned().into());
        }
        if paths.len() == 2 {
            return Err(format!("unexpected extra argument '{}'", Path::new(arg).display()).into());
        }
        paths.push(PathBuf::from(arg));
        i += 1;
    }
    let [path_a, path_b] = <[PathBuf; 2]>::try_from(paths)
        .map_err(|_| "verify-equiv requires <a.iff> <b.iff>".to_owned())?;
    let read = |path: &Path| -> Result<Vec<u8>, CliError> {
        let (file, len, hdr) = read_regular_file_prefix(path)?;
        Ok(read_bounded_iff_file(path, file, len, hdr)?)
    };
    for extra in &with {
        let bytes = read(extra)?;
        let arities = extract_bhav_arities_from_iff(&bytes)
            .map_err(|e| format!("{}: {e}", extra.display()))?;
        opts.extra_arities.extend(arities);
    }
    let (a, b) = (read(&path_a)?, read(&path_b)?);
    let report = check_iff_equivalent(&a, &b, &opts).map_err(|e| e.to_string())?;

    let mut out = OutWriter::stdout();
    let mut counts = [0usize; 4];
    for problem in &report.file_problems {
        out.writeln_str(&format!("FILE: {problem}"))?;
    }
    for r in &report.resources {
        let (slot, line) = match &r.verdict {
            ResourceVerdict::Identical => (0, None),
            ResourceVerdict::Equivalent { how } => (1, Some(format!("equivalent ({how})"))),
            ResourceVerdict::Different { why } => (2, Some(format!("DIFFERENT: {why}"))),
            ResourceVerdict::Inconclusive { why } => (3, Some(format!("INCONCLUSIVE: {why}"))),
        };
        counts[slot] += 1;
        let line = match line {
            Some(line) => Some(line),
            None if show_all || !r.notes.is_empty() => Some("identical".to_owned()),
            None => None,
        };
        if let Some(line) = line {
            let notes = if r.notes.is_empty() {
                String::new()
            } else {
                format!(" [notes: {}]", r.notes.join("; "))
            };
            out.writeln_str(&sanitize_single_line(&format!(
                "{}: {line}{notes}",
                r.name()
            )))?;
        }
    }
    let verdict = report.verdict();
    let word = match verdict {
        IffVerdict::Equivalent => "EQUIVALENT",
        IffVerdict::Different => "DIFFERENT",
        IffVerdict::Inconclusive => "INCONCLUSIVE",
    };
    out.writeln_str(&format!(
        "{word}: {} resources ({} identical, {} equivalent, {} different, {} inconclusive)",
        report.resources.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    ))?;
    out.flush()?;
    match verdict.exit_code() {
        0 => Ok(()),
        code => Err(CliError::Exit(code as u8)),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) | Err(CliError::BrokenPipe) => ExitCode::SUCCESS,
        Err(CliError::Message(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
        Err(CliError::Exit(code)) => ExitCode::from(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::iff::{ChunkData, write as write_iff};

    struct FailWithBrokenPipe {
        fail_after_bytes: usize,
        written: usize,
    }

    impl Write for FailWithBrokenPipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.written >= self.fail_after_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "simulated broken pipe",
                ));
            }
            let allow = (self.fail_after_bytes - self.written).min(buf.len());
            self.written += allow;
            Ok(allow)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.written >= self.fail_after_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "simulated broken pipe on flush",
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_does_not_erase_already_recorded_failures() {
        let counterexample = LiftSummary {
            counterexamples: 1,
            ..Default::default()
        };
        assert!(matches!(
            counterexample.into_result(true),
            Err(CliError::Message(msg)) if msg.contains("1 counterexample")
        ));

        let compile_err = LiftSummary {
            compile_errors: 1,
            ..Default::default()
        };
        assert!(matches!(
            compile_err.into_result(true),
            Err(CliError::Message(msg)) if msg.contains("1 compile error")
        ));

        let parse_err = LiftSummary {
            parse_errors: 1,
            ..Default::default()
        };
        assert!(matches!(
            parse_err.into_result(true),
            Err(CliError::Message(msg)) if msg.contains("failed to parse 1 BHAV")
        ));

        let clean = LiftSummary::default();
        assert!(matches!(clean.into_result(true), Err(CliError::BrokenPipe)));
    }

    #[test]
    fn bulk_parse_error_still_fails_even_when_stdout_hits_broken_pipe() {
        let iff_bytes = write_iff(&[ChunkData {
            tag: *b"BHAV",
            id: 4096,
            flags: 0,
            label: "Truncated".to_owned(),
            data: vec![0x02, 0x80], // truncated header
        }])
        .expect("write_iff");
        let iff = Iff::parse(iff_bytes).expect("Iff::parse");
        let chunks: Vec<_> = iff.all(b"BHAV").collect();
        let mut out = OutWriter::from_writer(FailWithBrokenPipe {
            fail_after_bytes: 0,
            written: 0,
        });
        let res = emit_lifted_bhavs(
            &iff,
            &chunks,
            "test.iff",
            None,
            false,
            LiftOptions::default(),
            false,
            &mut out,
        );
        assert!(matches!(
            res,
            Err(CliError::Message(msg)) if msg.contains("failed to parse 1 BHAV")
        ));
    }
}
