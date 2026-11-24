//! CLI entrypoint for expanding `.antics` v2 sources to `.air` (Antics IR),
//! compiling packs/objects, verifying safety, checking equivalence, and inspecting
//! lifted `.air` behavior trees from `.iff` files and `.far` archives.
//!
//! Usage:
//! - `sims-antics expand <file.antics> [--pack <pack_dir>]`
//! - `sims-antics check <pack_dir|file.antics> [--pack <pack_dir>]`
//! - `sims-antics verify <pack_dir|file.antics> [--pack <pack_dir>]`
//! - `sims-antics equiv <a.antics|a.air> <b.iff> --tree <id> [--pack <pack_dir>] [--ignore-last-result] [--ignore-frame-residue]`
//! - `sims-antics verify-equiv <a.iff> <b.iff> [--tree <id>] [--global <Global.iff>] [--semi-global <semi.iff>] [--my-attributes <n>] [--ignore-scratch-pointer] [--ignore-last-result] [--ignore-frame-residue] [--behavioral] [--param-domain <tree>:<index>=<set>]... [--v2-enum-domains <file.antics> [--pack <pack_dir>]] [--all]`
//!   (exit 0 equivalent, 1 different, 2 inconclusive or error). `--behavioral` compares
//!   without node numbers and frame storage (`docs/decisions/20261123-verifier-behavioral-mode.md`).
//!   The domain flags assume parameter values on entry and print every assumption
//!   (`docs/decisions/20261124-verifier-domains-and-speed.md`).
//!   Only files that differ in BHAV bodies alone are compared; DIFFERENT is reported only when both trees, run in
//!   sims-vm from the same world, end differently; EQUIVALENT assumes neither run
//!   exhausts the VM's instruction budget.
//! - `sims-antics lift <file.iff|archive.far> [--entry <name.iff>] [--tree <id>] [--list] [--filter <substr>] [--raw] [--relax-padding] [--no-comments] [--verify-equiv] [--globals <Global.iff|archive.far[!member.iff]>] [--semiglobal <file.iff|archive.far!member.iff>] [--no-catalog]`
//! - `sims-antics build <pack_dir> --out <dir>`
//! - `sims-antics fmt [--check] [--stdout] <path...>` (canonical `.antics` v2 layout)
//! - `sims-antics lift --v2 <file.iff|archive.far> [--entry <name.iff>] [--install <dir>]` (decompile to `.antics` v2)

use sims_antics::arity_catalog::{
    AritySource, FileArities, FileIdentity, GLOBAL_TREES, KeptBhavs, ResolvedArities,
    SEMI_GLOBAL_TREES, bhav_arities_in_range, vm_kept_bhavs,
};
use sims_antics::ir::BhavChunk;
use sims_antics::{
    EquivOptions, EquivResult, IffEquivOptions, IffVerdict, LiftOptions, LiveOptions,
    ResourceVerdict, StructuredLiftOutcome, UnsupportedHeaderReason, callee_scope_name,
    check_bhav_equivalent_with_arities, check_iff_equivalent, compile_air, compile_pack_dir,
    compile_standalone_pack_dir, compile_v2_object_file, expand_v2_to_air,
    extract_lift_symbols_from_iff,
    lift::{format_bhav_labeled_air_with_context, format_bhav_raw_air, lift_bhav},
    lift_bhav_structured_with_arities, lift_bhav_structured_with_context,
    structured_lift_header_check,
    v2::{DecompileV2Options, TreeCheck, decompile_iff_to_v2_report, is_v2_source},
    verify::{
        ParamDomain, check_tree_with_callees, extract_bhav_from_iff, library_argless_from_iff,
        library_arities_from_iff, parse_param_domain_arg, single_file_problem,
        v2_enum_param_domains,
    },
    verify_object_safety, verify_pack_safety,
};
use sims_formats::far;
use sims_formats::iff::Iff;
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

/// The model assumption every EQUIVALENT verdict carries: the checkers compare what the
/// trees do, not how many instructions they take, and `sims-vm` faults a run that
/// exceeds its per-tick instruction budget (`interpreter.rs:494`).
const BUDGET_ASSUMPTION: &str = ", assuming neither run exhausts the VM instruction budget";

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         sims-antics expand <file.antics> [--pack <pack_dir>]\n  \
         sims-antics check <pack_dir|file.antics> [--pack <pack_dir>]\n  \
         sims-antics verify <pack_dir|file.antics> [--pack <pack_dir>]\n  \
         sims-antics equiv <a.antics|a.air> <b.iff> --tree <id> [--pack <pack_dir>] [--ignore-last-result] [--ignore-frame-residue]\n  \
         sims-antics verify-equiv <a.iff> <b.iff> [--tree <id>] [--global <Global.iff>] [--semi-global <semi.iff>] [--my-attributes <n>] [--ignore-scratch-pointer] [--ignore-last-result] [--ignore-frame-residue] [--behavioral] [--param-domain <tree>:<index>=<set>]... [--v2-enum-domains <file.antics> [--pack <pack_dir>]] [--all]   (exit 0 equivalent, 1 different, 2 inconclusive)\n    \
             (files must differ in BHAV bodies alone; DIFFERENT needs a sims-vm replay that ends differently;\n    \
              EQUIVALENT assumes neither run exhausts the VM instruction budget; the same holds for equiv;\n    \
              a tree is EQUIVALENT only if every tree it may run is the same bytes or proven equivalent too;\n    \
              --ignore-last-result and --ignore-frame-residue relax the named tree only: the trees it may run are\n    \
              proven with both observed, since a primitive may run one as an object use whose return records them;\n    \
              only private trees, 4096 to 8191, are run from an object's file: --global and --semi-global\n    \
              name the libraries calls to 256-4095 and 8192 and up run; they are trusted to be the game's,\n    \
              and two that disagree on a tree are refused;\n    \
              --my-attributes is the attribute count of whatever runs as me, which call_as may make another\n    \
              object than the one whose file holds the tree: give the smallest count of anything that runs it;\n    \
              --behavioral ignores node numbers and frame storage (implies --ignore-frame-residue) and holds\n    \
              only for runs that start and end under one build, see docs/decisions/20261123-verifier-behavioral-mode.md;\n    \
              --param-domain assumes param[<index>] of <tree> is in <set> (1,4,9 or 0..=3) on entry, and\n    \
              --v2-enum-domains assumes it for every enum-typed parameter of the file's trees: EQUIVALENT then\n    \
              holds only for such runs; every assumption is printed, with how the files' own calls stand against it)\n  \
         sims-antics lift <file.iff|archive.far> [--entry <name.iff>] [--tree <id>] [--list] [--filter <substr>] [--raw] [--relax-padding] [--no-comments] [--verify-equiv] [--globals <Global.iff|archive.far[!member.iff]>] [--semiglobal <file.iff|archive.far!member.iff>] [--no-catalog]   (lifts to .air Antics IR)\n  \
         sims-antics lift --v2 <file.iff|archive.far> [--entry <name.iff>] [--install <dir>] [--globals ...] [--semiglobal ...] [--no-catalog]   (decompiles an object to checked .antics v2)\n  \
         sims-antics build <pack_dir> --out <dir>\n  \
         sims-antics fmt [--check] [--stdout] <file.antics|dir...>   (canonical .antics v2 layout; --check exits 1 if any file would change)\n\
         lift arities: inside an install, global and semi-global callee arities come from its catalog;\n  \
         --globals/--semiglobal replace one range; --no-catalog disables catalog lookup (private arities\n  \
         from the file only). Equivalent is relative to the arities shown in the `// arity sources:` header.\n  \
         Assumes the install is not modified during a run; supported hosts are macOS and Linux (x86_64/aarch64)."
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

fn format_ambiguous_matches(what: &str, query: &str, matches: &[&far::Entry]) -> String {
    const MAX_LISTED: usize = 20;
    let mut names: Vec<String> = matches
        .iter()
        .map(|e| sanitize_single_line(&e.name))
        .collect();
    names.sort();
    let mut msg = format!(
        "ambiguous {what} '{}' matched {} FAR entries:",
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
        return Err(format_ambiguous_matches("--entry", query, &exact));
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
        return Err(format_ambiguous_matches("--entry", query, &ci_exact));
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
            return Err(format_ambiguous_matches("--entry", query, &seg_suffix));
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
            return Err(format_ambiguous_matches("--entry", query, &general_suffix));
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

/// The fallback reason for `outcome`, naming the node, callee, or target that made the
/// header/target check refuse the tree and, for an unknown-arity callee, which arity
/// source was missing.
fn detailed_lift_reason(
    outcome: &StructuredLiftOutcome,
    bhav: &BhavChunk,
    resolved: &ResolvedArities,
) -> String {
    let base = lift_reason(outcome).unwrap_or_else(|| "unknown".to_owned());
    if !matches!(outcome, StructuredLiftOutcome::UnsupportedHeaderOrTarget) {
        return base;
    }
    let Err(reason) = structured_lift_header_check(bhav, Some(&resolved.arities)) else {
        return base;
    };
    let mut out = format!("{base}: {reason}");
    if let UnsupportedHeaderReason::UnknownArityCallee { callee, .. } = reason
        && let Some(part) = resolved.part_for(callee)
    {
        let scope = callee_scope_name(callee);
        let hint = match &part.source {
            AritySource::Missing(why) => {
                let flag = if GLOBAL_TREES.contains(&callee) {
                    "; pass --globals <Global.iff>"
                } else if SEMI_GLOBAL_TREES.contains(&callee) {
                    "; pass --semiglobal <semiglobal.iff>"
                } else {
                    "; pass --no-catalog to use this file's own private trees"
                };
                format!("; {scope} arities not loaded ({why}){flag}")
            }
            AritySource::ThisFile | AritySource::ThisFileBecause(_) => format!(
                "; no usable arity for tree {callee} in this file (absent, or duplicate chunks disagree)"
            ),
            AritySource::Catalog(location) | AritySource::Explicit(location) => format!(
                "; no usable arity for tree {callee} in {location} (absent, or duplicate chunks disagree)"
            ),
        };
        out.push_str(&sanitize_single_line(&hint));
    }
    out
}

/// A compact fallback reason for the `--list` status column.
fn short_lift_reason(
    outcome: &StructuredLiftOutcome,
    bhav: &BhavChunk,
    resolved: &ResolvedArities,
) -> String {
    if matches!(outcome, StructuredLiftOutcome::UnsupportedHeaderOrTarget)
        && let Err(reason) = structured_lift_header_check(bhav, Some(&resolved.arities))
    {
        return match reason {
            UnsupportedHeaderReason::Format(format) => format!("format {format:#06x}"),
            UnsupportedHeaderReason::TooManyInstructions(n) => format!("{n} instructions"),
            UnsupportedHeaderReason::IndexedEntry => "kind 4 indexed entry".to_owned(),
            UnsupportedHeaderReason::TargetOutOfBounds { node, target } => {
                format!("node {node} target {target} out of bounds")
            }
            UnsupportedHeaderReason::ExtendedArityCallee { callee, arity, .. } => format!(
                "calls {} {callee} with {arity} args",
                callee_scope_name(callee)
            ),
            UnsupportedHeaderReason::UnknownArityCallee { callee, .. } => format!(
                "calls {} {callee}, arity unknown",
                callee_scope_name(callee)
            ),
        };
    }
    lift_reason(outcome).unwrap_or_else(|| "unknown".to_owned())
}

/// Where `lift` looks for callee arities.
#[derive(Default)]
struct ArityOptions {
    no_catalog: bool,
    /// `--install <dir>`: the install whose catalog to use instead of one found above the input.
    install: Option<PathBuf>,
    globals: Option<String>,
    semi_global: Option<String>,
}

/// Merges callee arities for the lifted IFF. With `--no-catalog`: private trees from the
/// IFF itself. Otherwise from the enclosing install's catalog, but only when the file is
/// positively identified as one of its entries; with no inferred role every range is
/// unknown. Explicit `--globals` / `--semiglobal` overrides apply last.
fn resolve_lift_arities(
    iff: &Iff,
    input_path: &Path,
    input_identity: Result<&FileIdentity, &str>,
    member: Option<(&str, u64)>,
    opts: &ArityOptions,
) -> Result<FileArities, CliError> {
    let mut resolved = if opts.no_catalog {
        FileArities::private_only(iff, "--no-catalog")
    } else {
        catalog_arities(input_path, input_identity, member, opts.install.as_deref())
    };
    if let Some(spec) = &opts.globals {
        let (globals_iff, label) = read_override_iff(spec, Some("Global.iff"), "--globals")?;
        resolved.set_global(
            &bhav_arities_in_range(&globals_iff, GLOBAL_TREES),
            &AritySource::Explicit(label),
        );
    }
    if let Some(spec) = &opts.semi_global {
        let (semi_iff, label) = read_override_iff(spec, None, "--semiglobal")?;
        resolved.set_semi_global(
            &bhav_arities_in_range(&semi_iff, SEMI_GLOBAL_TREES),
            &AritySource::Explicit(label),
        );
    }
    Ok(resolved)
}

/// Arities from the enclosing install's catalog, for a positively identified file only.
#[cfg(feature = "install-catalog")]
fn catalog_arities(
    input_path: &Path,
    input_identity: Result<&FileIdentity, &str>,
    member: Option<(&str, u64)>,
    install: Option<&Path>,
) -> FileArities {
    use sims_antics::arity_catalog::InstallArityCatalog;
    let root = match install {
        Some(dir) => Some(dir.to_path_buf()),
        None => InstallArityCatalog::find_install_root(input_path),
    };
    match root {
        Some(root) => match InstallArityCatalog::load(&root) {
            Ok(mut catalog) => match input_identity {
                Ok(identity) => {
                    let resource = catalog.resource_for(input_path, identity, member);
                    catalog.resolve(resource)
                }
                Err(why) => FileArities::no_role(&format!(
                    "could not identify the opened file: {}",
                    sanitize_single_line(why)
                )),
            },
            Err(err) => FileArities::no_role(&format!(
                "install catalog failed to load: {}",
                sanitize_single_line(&err)
            )),
        },
        None => FileArities::no_role("no Sims install found above the input"),
    }
}

/// Without the `install-catalog` feature there is no install catalog: no inferred role.
#[cfg(not(feature = "install-catalog"))]
fn catalog_arities(
    _input_path: &Path,
    _input_identity: Result<&FileIdentity, &str>,
    _member: Option<(&str, u64)>,
    _install: Option<&Path>,
) -> FileArities {
    FileArities::no_role("built without the install-catalog feature")
}

/// Reads an IFF named by an arity override: a `.iff` file, or a FAR archive member given
/// as `<archive.far>!<member.iff>` (`default_member` when the archive has no `!member`).
fn read_override_iff(
    spec: &str,
    default_member: Option<&str>,
    flag: &str,
) -> Result<(Iff, String), CliError> {
    let with_flag = |e: CliError| match e {
        CliError::Message(msg) => CliError::Message(format!("{flag}: {msg}")),
        other => other,
    };
    // An existing file named by the whole spec wins. Otherwise `<archive>.far!<member>`
    // splits at the first `.far!` (ASCII case-insensitive) whose prefix is an existing
    // file, so both directory names and member names may contain `!`.
    let (path, member) = if Path::new(spec).is_file() {
        (Path::new(spec), None)
    } else {
        let lower = spec.to_ascii_lowercase();
        lower
            .match_indices(".far!")
            .map(|(at, _)| at)
            .find(|&at| Path::new(&spec[..at + 4]).is_file())
            .map_or((Path::new(spec), None), |at| {
                (Path::new(&spec[..at + 4]), Some(&spec[at + 5..]))
            })
    };
    if member == Some("") {
        return Err(format!(
            "{flag}: empty member name after '!' in '{}'",
            sanitize_single_line(spec)
        )
        .into());
    }
    let safe_path = sanitize_path_display(path);
    let (mut file, file_len, header) =
        read_regular_file_prefix(path).map_err(|e| format!("{flag}: {e}"))?;
    let is_far = header.len() >= 8 && &header[..8] == b"FAR!byAZ";
    let (bytes, label) = if is_far {
        let query = member.or(default_member).ok_or_else(|| {
            format!(
                "{flag}: {safe_path} is a FAR archive; name a member as <archive.far>!<member.iff>"
            )
        })?;
        let (manifest_off, entries) =
            read_far_directory(&mut file, file_len, &header, path).map_err(with_flag)?;
        let entry =
            resolve_far_entry_exact(&entries, query, path).map_err(|e| format!("{flag}: {e}"))?;
        let bytes = read_far_member(&mut file, entry, manifest_off, path).map_err(with_flag)?;
        let label = format!("{safe_path}!{}", sanitize_single_line(&entry.name));
        (bytes, label)
    } else {
        if member.is_some() {
            return Err(format!("{flag}: {safe_path} is not a FAR archive").into());
        }
        (
            read_bounded_iff_file(path, &mut file, file_len, header)
                .map_err(|e| format!("{flag}: {e}"))?,
            safe_path,
        )
    };
    let iff = Iff::parse(bytes).map_err(|e| format!("{flag}: failed to parse IFF {label}: {e}"))?;
    Ok((iff, label))
}

/// Resolves an arity-override member strictly: the exact name, else the whole name or its
/// last path component compared ASCII case-insensitively. Never a suffix match, and an
/// ambiguous tier is an error.
fn resolve_far_entry_exact<'a>(
    entries: &'a [far::Entry],
    query: &str,
    archive_path: &Path,
) -> Result<&'a far::Entry, String> {
    let last = |name: &str| name.rsplit(['/', '\\']).next().unwrap_or(name).to_owned();
    let tiers: [&dyn Fn(&far::Entry) -> bool; 3] = [
        &|e| e.name == query,
        &|e| e.name.eq_ignore_ascii_case(query),
        &|e| last(&e.name).eq_ignore_ascii_case(query),
    ];
    for tier in tiers {
        let matched: Vec<&far::Entry> = entries.iter().filter(|e| tier(e)).collect();
        match matched.len() {
            0 => {}
            1 => return Ok(matched[0]),
            _ => return Err(format_ambiguous_matches("FAR member", query, &matched)),
        }
    }
    Err(format!(
        "FAR member '{}' not found in {}",
        sanitize_single_line(query),
        sanitize_path_display(archive_path)
    ))
}

/// Reads a FAR archive's directory: the manifest offset and its entries.
fn read_far_directory(
    file: &mut std::fs::File,
    file_len: u64,
    header: &[u8],
    path: &Path,
) -> Result<(u32, Vec<far::Entry>), CliError> {
    let safe_path = sanitize_path_display(path);
    if header.len() < 16 {
        return Err(
            format!("invalid FAR archive {safe_path}: file shorter than 16-byte header").into(),
        );
    }
    let manifest_off = far::manifest_offset(header, file_len)
        .map_err(|e| format!("failed to parse FAR header in {safe_path}: {e}"))?;
    let dir_len = far::directory_len(manifest_off, file_len)
        .map_err(|e| format!("invalid FAR directory in {safe_path}: {e}"))?;
    file.seek(SeekFrom::Start(u64::from(manifest_off)))
        .map_err(|e| format!("failed to seek {safe_path}: {e}"))?;
    let mut dir_buf = vec![0u8; dir_len];
    file.read_exact(&mut dir_buf)
        .map_err(|e| format!("failed to read FAR directory in {safe_path}: {e}"))?;
    let entries = far::directory(&dir_buf, file_len)
        .map_err(|e| format!("failed to parse FAR directory in {safe_path}: {e}"))?;
    Ok((manifest_off, entries))
}

/// Reads the uncompressed FAR member `entry`.
fn read_far_member(
    file: &mut std::fs::File,
    entry: &far::Entry,
    manifest_off: u32,
    path: &Path,
) -> Result<Vec<u8>, CliError> {
    let safe_path = sanitize_path_display(path);
    let safe_entry = sanitize_single_line(&entry.name);
    if !entry.within_payload(manifest_off) {
        return Err(
            format!("FAR entry '{safe_entry}' in {safe_path} has out-of-bounds payload").into(),
        );
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
        .map_err(|e| format!("failed to seek {safe_path}: {e}"))?;
    let mut member_bytes = vec![0u8; entry.stored_size as usize];
    file.read_exact(&mut member_bytes)
        .map_err(|e| format!("failed to read FAR entry '{safe_entry}' from {safe_path}: {e}"))?;
    Ok(member_bytes)
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
    file: &mut std::fs::File,
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
    Read::by_ref(file)
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
    resolved: Option<&FileArities>,
    duplicates: &std::collections::BTreeMap<u16, usize>,
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

    let symbols = if lift_opts.annotate_comments {
        Some(extract_lift_symbols_from_iff(iff))
    } else {
        None
    };

    let mut summary = LiftSummary::default();
    let mut broken_pipe = false;

    let tree_ids: Vec<u16> = selected_chunks.iter().map(|c| c.id).collect();
    // `resolved` is `None` for `--raw`, which does not use callee arities. A broken pipe
    // here still lets the loop below count parse errors before it stops.
    if let Some(resolved) = resolved {
        match out.writeln_str(&format!(
            "// arity sources: {}; lifting and --verify-equiv assume these callee arities (an \
             override, or a lot whose loaded callee versions differ, may change them)",
            sanitize_single_line(&resolved.describe_for(&tree_ids))
        )) {
            Ok(()) => {}
            Err(CliError::BrokenPipe) => broken_pipe = true,
            Err(other) => return Err(other),
        }
    }
    let selected_duplicates: std::collections::BTreeMap<u16, usize> = duplicates
        .iter()
        .filter(|(id, _)| tree_ids.contains(id))
        .map(|(&id, &count)| (id, count))
        .collect();
    if !broken_pipe && let Some(note) = duplicates_note(&selected_duplicates) {
        match out.writeln_str(&note) {
            Ok(()) => {}
            Err(CliError::BrokenPipe) => broken_pipe = true,
            Err(other) => return Err(other),
        }
    }

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

        // Callee arities depend on the scope the calling tree runs in.
        let tree_arities = resolved.map(|r| r.for_tree(bhav.id));
        let arities = tree_arities.map(|r| &r.arities);
        let outcome =
            lift_bhav_structured_with_context(&bhav, lift_opts, arities, symbols.as_ref());
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
                let reason = match tree_arities {
                    Some(resolved) => detailed_lift_reason(&other, &bhav, resolved),
                    None => lift_reason(&other).unwrap_or_else(|| "unknown".to_owned()),
                };
                write_or_break!(out.writeln_str(&format!(
                    "// tree {} \"{}\": structured lift fallback ({reason}); showing decoded labeled-CFG .air (Antics IR)",
                    bhav.id,
                    sanitize_single_line(&bhav.label)
                )));
                let labeled_src = format_bhav_labeled_air_with_context(
                    &bhav,
                    lift_opts,
                    arities,
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
                            arities,
                        ) {
                            EquivResult::Equivalent {
                                states_checked,
                                paths_verified,
                            } => format!(
                                "// verify-equiv: Equivalent ({states_checked} states checked, {paths_verified} paths verified){}",
                                if arities.is_some() {
                                    " under the callee arities above"
                                } else {
                                    ""
                                }
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
    let mut v2 = false;
    let mut arity_opts = ArityOptions::default();
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
                    "--no-catalog" => {
                        arity_opts.no_catalog = true;
                        i += 1;
                        continue;
                    }
                    "--v2" => {
                        v2 = true;
                        i += 1;
                        continue;
                    }
                    "--install" => {
                        if arity_opts.install.is_some() {
                            return Err("duplicate --install flag".to_owned().into());
                        }
                        let val = args
                            .get(i + 1)
                            .ok_or_else(|| "missing directory after --install".to_owned())?;
                        arity_opts.install = Some(PathBuf::from(val));
                        i += 2;
                        continue;
                    }
                    "--globals" | "--semiglobal" => {
                        let slot = if s == "--globals" {
                            &mut arity_opts.globals
                        } else {
                            &mut arity_opts.semi_global
                        };
                        if slot.is_some() {
                            return Err(format!("duplicate {s} flag").into());
                        }
                        let val = args
                            .get(i + 1)
                            .ok_or_else(|| format!("missing file after {s}"))?;
                        *slot = Some(os_arg_to_str(val, &format!("{s} value"))?.to_owned());
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

    if v2
        && (list
            || tree_id.is_some()
            || filter.is_some()
            || raw
            || relax_padding
            || no_comments
            || verify_equiv)
    {
        return Err(
            "--v2 cannot be combined with --list, --tree, --filter, --raw, --relax-padding, --no-comments, or --verify-equiv (it checks every tree itself)"
                .to_owned()
                .into(),
        );
    }
    if arity_opts.install.is_some() && (raw || arity_opts.no_catalog) {
        return Err("--install cannot be combined with --raw or --no-catalog"
            .to_owned()
            .into());
    }
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
    if raw
        && (arity_opts.no_catalog
            || arity_opts.globals.is_some()
            || arity_opts.semi_global.is_some())
    {
        return Err(
            "--globals, --semiglobal, and --no-catalog cannot be combined with --raw (raw output does not use callee arities)"
                .to_owned()
                .into(),
        );
    }
    let lift_opts = LiftOptions {
        strict_padding: !relax_padding,
        annotate_comments: !no_comments,
    };

    let safe_input_path = sanitize_path_display(&input_path);
    // `file` stays open until the catalog lookup is done: while it is open its inode
    // cannot be freed and reused by another file.
    let (mut file, file_len, header) = read_regular_file_prefix(&input_path)?;
    // The identity of the file the bytes are read from (fstat on this handle), so the
    // catalog lookup can never match a different file now at this path.
    let input_identity = FileIdentity::of_open(&file, &input_path).map_err(|e| e.to_string());
    let has_far_ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("far"));
    let has_far_magic = header.len() >= 8 && &header[..8] == b"FAR!byAZ";
    let is_far = has_far_ext || has_far_magic;

    let mut out = OutWriter::stdout();

    let (iff_bytes, source_label, far_member) = if is_far {
        let (manifest_off, entries) =
            read_far_directory(&mut file, file_len, &header, &input_path)?;
        match entry_name {
            None => {
                if arity_opts.no_catalog
                    || arity_opts.globals.is_some()
                    || arity_opts.semi_global.is_some()
                {
                    return Err("--globals, --semiglobal, and --no-catalog need --entry <name.iff> (listing an archive's entries does not lift trees)"
                        .to_owned()
                        .into());
                }
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
                let member_bytes = read_far_member(&mut file, entry, manifest_off, &input_path)?;
                let label = format!("{safe_input_path}!{}", sanitize_single_line(&entry.name));
                let member = (entry.name.clone(), u64::from(entry.offset));
                (member_bytes, label, Some(member))
            }
        }
    } else {
        if entry_name.is_some() {
            return Err(format!(
                "--entry is only valid for .far archives ({safe_input_path} is not a FAR archive)"
            )
            .into());
        }
        let bytes = read_bounded_iff_file(&input_path, &mut file, file_len, header)?;
        (bytes, safe_input_path, None)
    };

    let iff =
        Iff::parse(iff_bytes).map_err(|e| format!("failed to parse IFF {source_label}: {e}"))?;

    let KeptBhavs {
        chunks: bhav_chunks,
        duplicates,
    } = vm_kept_bhavs(&iff);

    // Resolved after tree selection (so a bad override never masks a not-found tree), and
    // skipped for `--raw`, which does not use arities, or when there is nothing to lift and
    // no explicit override to validate.
    let has_overrides = arity_opts.globals.is_some() || arity_opts.semi_global.is_some();
    let nothing_to_lift = bhav_chunks.is_empty();
    let resolve_arities = || -> Result<Option<FileArities>, CliError> {
        if raw || (nothing_to_lift && !has_overrides) {
            return Ok(None);
        }
        resolve_lift_arities(
            &iff,
            &input_path,
            input_identity.as_ref().map_err(String::as_str),
            far_member
                .as_ref()
                .map(|(name, offset)| (name.as_str(), *offset)),
            &arity_opts,
        )
        .map(Some)
    };

    if v2 {
        let resolved = resolve_arities()?;
        return lift_v2(&iff, resolved.as_ref(), &source_label, &mut out);
    }

    if list {
        let resolved = resolve_arities()?;
        let filter_lower = filter.as_deref().map(str::to_lowercase);
        if let Some(resolved) = &resolved {
            let listed: Vec<u16> = bhav_chunks
                .iter()
                .filter(|c| {
                    filter_lower
                        .as_ref()
                        .is_none_or(|needle| bhav_matches_filter(c.id, &c.label, needle))
                })
                .map(|c| c.id)
                .collect();
            out.writeln_str(&format!(
                "// arity sources: {}",
                sanitize_single_line(&resolved.describe_for(&listed))
            ))?;
        }
        if let Some(note) = duplicates_note(&duplicates) {
            out.writeln_str(&note)?;
        }
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
                    let tree_arities = resolved.as_ref().map(|r| r.for_tree(bhav.id));
                    let status = match lift_bhav_structured_with_arities(
                        &bhav,
                        list_opts,
                        tree_arities.map(|r| &r.arities),
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
                            match tree_arities {
                                Some(resolved) => short_lift_reason(&other, &bhav, resolved),
                                None => lift_reason(&other).unwrap_or_else(|| "unknown".to_owned()),
                            }
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
        let chunk = bhav_chunks
            .iter()
            .copied()
            .find(|c| c.id == target_id)
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

    let resolved = resolve_arities()?;
    emit_lifted_bhavs(
        &iff,
        &selected_chunks,
        &source_label,
        tree_id,
        raw,
        resolved.as_ref(),
        &duplicates,
        lift_opts,
        verify_equiv,
        &mut out,
    )
}

/// A comment naming IDs shared by several BHAV chunks, if any.
fn duplicates_note(duplicates: &std::collections::BTreeMap<u16, usize>) -> Option<String> {
    if duplicates.is_empty() {
        return None;
    }
    let ids: Vec<String> = duplicates
        .iter()
        .map(|(id, count)| format!("{id} ({count} chunks)"))
        .collect();
    Some(format!(
        "// duplicate BHAV IDs: {}; each shows the last chunk in file order, the one the VM keeps",
        ids.join(", ")
    ))
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
            let mut live_opts = LiveOptions::default();
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
                        "--ignore-last-result" => {
                            live_opts.observe_last_result = false;
                            i += 1;
                            continue;
                        }
                        "--ignore-frame-residue" => {
                            live_opts.observe_frame_residue = false;
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
            // Side A: the compiled source, with its IFF when it is a v2 object.
            let (behaviors, file_a) = if is_v2_source(&antics_src) {
                let object = compile_v2_object_file(&antics_path, pack_dir.as_deref())
                    .map_err(|e| e.to_string())?;
                (object.behaviors, Some(object.iff_bytes))
            } else {
                let module = compile_air(&antics_src).map_err(|e| e.to_string())?;
                (module.behaviors, None)
            };
            if !behaviors.contains_key(&tree_id) {
                return Err(
                    format!("{} does not define tree #{tree_id}", antics_path.display()).into(),
                );
            }
            let (mut iff_file, iff_len, iff_hdr) = read_regular_file_prefix(&iff_path)?;
            let iff_bytes = read_bounded_iff_file(&iff_path, &mut iff_file, iff_len, iff_hdr)?;
            // The tree compared must be the one every loader runs: one chunk per type
            // and ID, and a usable map that selects it.
            let parsed = Iff::parse(iff_bytes.clone()).map_err(|e| e.to_string())?;
            if let Some(problem) = single_file_problem(&parsed) {
                return Err(format!("INCONCLUSIVE on tree #{tree_id}: {problem}").into());
            }
            extract_bhav_from_iff(&iff_bytes, tree_id).map_err(|e| e.to_string())?;
            // Side B: every tree of the IFF (one chunk per ID, checked above).
            let trees_b: std::collections::BTreeMap<u16, _> = parsed
                .chunks
                .iter()
                .filter(|c| &c.tag == b"BHAV")
                .filter_map(|c| {
                    lift_bhav(c.id, &c.label, parsed.data(c))
                        .ok()
                        .map(|bhav| (c.id, bhav))
                })
                .collect();
            // Each side calls the trees of its own file: the compiled source's for A,
            // the IFF's for B. A callee only one side defines has an unknown argument
            // count on the other, which the checkers treat as a possible difference.
            // As the VM's loader keeps them (`program.rs:588`).
            let arities_b: std::collections::BTreeMap<u16, usize> =
                library_arities_from_iff(&iff_bytes)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(id, n)| Some((id, n?)))
                    .collect();
            let arities_a: std::collections::BTreeMap<u16, usize> = behaviors
                .iter()
                .map(|(&id, bhav)| {
                    let argc = match bhav.format {
                        0x8000 => 4,
                        0x8001 => usize::from(bhav.args).min(4),
                        _ => usize::from(bhav.args),
                    };
                    (id, argc)
                })
                .collect();
            // The tree, and every tree it may run, must behave the same; a run that may
            // read the file's other resources needs two IFFs that differ in BHAV bodies
            // alone.
            let (method, result) = check_tree_with_callees(
                tree_id,
                &behaviors,
                file_a.as_deref(),
                &trees_b,
                Some(&iff_bytes),
                (&arities_a, &arities_b),
                &EquivOptions::default(),
                &live_opts,
            );
            match result {
                EquivResult::Equivalent {
                    states_checked,
                    paths_verified,
                } => write_stdout_line(&format!(
                    "EQUIVALENT: tree #{tree_id} ({method}: {states_checked} states checked, {paths_verified} paths verified){BUDGET_ASSUMPTION}"
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
        "fmt" => cmd_fmt(&args),
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
/// `sims-antics fmt [--check] [--stdout] <path...>`: canonical `.antics` v2 layout.
/// `lift --v2`: prints the object as checked `.antics` v2 and a per-tree summary on
/// stderr. Exits 1 when a tree differs even written as `raw`.
fn lift_v2(
    iff: &Iff,
    resolved: Option<&FileArities>,
    source_label: &str,
    out: &mut OutWriter<impl Write>,
) -> Result<(), CliError> {
    // Library arities are the same for every private caller; private ones come from the file.
    let library_arities = resolved
        .map(|r| {
            r.for_tree(4096)
                .arities
                .iter()
                .filter(|&(id, _)| !(4096..=8191).contains(id))
                .map(|(&id, &n)| (id, n))
                .collect()
        })
        .unwrap_or_default();
    let options = DecompileV2Options {
        library_arities,
        ..DecompileV2Options::default()
    };
    let report = decompile_iff_to_v2_report(iff, &options)
        .map_err(|e| format!("failed to decompile {source_label}: {e}"))?;
    out.write_str(&report.source)?;
    out.flush()?;
    let mut counts: [usize; 5] = [0; 5];
    let (mut idiomatic, mut raw_statements) = (0usize, 0usize);
    for t in &report.trees {
        let slot = match &t.check {
            TreeCheck::Identical => 0,
            TreeCheck::Equivalent => 1,
            TreeCheck::Inconclusive(_) | TreeCheck::NotChecked => 2,
            TreeCheck::Omitted(_) => 3,
            TreeCheck::Different(_) => 4,
        };
        counts[slot] += 1;
        if t.style == sims_antics::v2::TreeStyle::Idiomatic {
            idiomatic += 1;
            raw_statements += t.raw_statements;
        }
    }
    eprintln!(
        "decompiled {} tree(s) from {source_label}: {} identical, {} equivalent, {} inconclusive, {} omitted, {} different; {idiomatic} idiomatic ({raw_statements} raw statement(s)), {} raw; {} init/main tree(s) written as plain trees",
        report.trees.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        report.trees.len() - idiomatic - counts[3],
        report
            .trees
            .iter()
            .filter(|t| t.standalone_lifecycle)
            .count(),
    );
    for t in &report.trees {
        let why = match &t.check {
            TreeCheck::Inconclusive(why) => format!("inconclusive: {why}"),
            TreeCheck::Omitted(why) => format!("omitted: {why}"),
            TreeCheck::Different(why) => format!("different: {why}"),
            _ => continue,
        };
        eprintln!("  tree {}: {}", t.id, sanitize_single_line(&why));
    }
    if !report.resources_not_reproduced.is_empty() {
        eprintln!(
            "note: {} resource chunk(s) are not reproduced byte for byte ({}); trees that read them are checked as code only",
            report.resources_not_reproduced.len(),
            report.resources_not_reproduced.join(", ")
        );
    }
    if report.counterexamples() > 0 {
        return Err(CliError::Exit(1));
    }
    Ok(())
}

fn cmd_fmt(args: &[OsString]) -> Result<(), CliError> {
    let mut check = false;
    let mut to_stdout = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut end_of_flags = false;
    for arg in &args[1..] {
        if !end_of_flags && let Some(s) = arg.to_str() {
            match s {
                "--check" => {
                    check = true;
                    continue;
                }
                "--stdout" => {
                    to_stdout = true;
                    continue;
                }
                "--" => {
                    end_of_flags = true;
                    continue;
                }
                "-h" | "--help" => {
                    print_usage();
                    return Ok(());
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag '{other}'").into());
                }
                _ => {}
            }
        }
        paths.push(PathBuf::from(arg));
    }
    if paths.is_empty() {
        return Err("fmt requires at least one <file.antics|dir>"
            .to_owned()
            .into());
    }
    if check && to_stdout {
        return Err("--check cannot be combined with --stdout".to_owned().into());
    }
    let files = sims_antics::v2::fmt::collect_antics_files(&paths).map_err(|e| e.to_string())?;
    if to_stdout && files.len() != 1 {
        return Err(format!("--stdout needs exactly one file ({} given)", files.len()).into());
    }
    let mut out = OutWriter::stdout();
    let mut unformatted = 0usize;
    for file in &files {
        let shown = sanitize_path_display(file);
        let source =
            std::fs::read_to_string(file).map_err(|e| format!("failed to read {shown}: {e}"))?;
        let formatted = match sims_antics::v2::format_v2_source(&source) {
            Ok(f) => f,
            // A `.air` (Antics IR) file shares the extension; it is not v2 and is left alone.
            Err(_) if !is_v2_source(&source) => {
                eprintln!("note: skipping {shown} (not .antics v2)");
                continue;
            }
            Err(e) => return Err(format!("{shown}: {e}").into()),
        };
        if to_stdout {
            out.write_str(&formatted)?;
            continue;
        }
        if formatted == source {
            continue;
        }
        unformatted += 1;
        if check {
            out.writeln_str(&shown)?;
        } else {
            std::fs::write(file, formatted).map_err(|e| format!("failed to write {shown}: {e}"))?;
        }
    }
    out.flush()?;
    if check && unformatted > 0 {
        eprintln!("{unformatted} file(s) need formatting");
        return Err(CliError::Exit(1));
    }
    Ok(())
}

fn cmd_verify_equiv(args: &[OsString]) -> Result<(), CliError> {
    let mut paths: Vec<PathBuf> = Vec::new();
    // Library files and whether each is a semi-global one.
    let mut with: Vec<(PathBuf, bool)> = Vec::new();
    let mut opts = IffEquivOptions::default();
    let mut show_all = false;
    // Parameter domains: `(tree, index, domain, where it came from)`.
    let mut assumed: Vec<(u16, u16, ParamDomain, String)> = Vec::new();
    let mut enum_sources: Vec<PathBuf> = Vec::new();
    let mut pack: Option<PathBuf> = None;
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
                "--global" => {
                    with.push((PathBuf::from(value(s)?), false));
                    i += 2;
                    continue;
                }
                "--semi-global" => {
                    with.push((PathBuf::from(value(s)?), true));
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
                            "--my-attributes must be at least {} (whatever runs as me has that many)",
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
                "--ignore-frame-residue" => {
                    opts.live.observe_frame_residue = false;
                    i += 1;
                    continue;
                }
                "--ignore-last-result" => {
                    opts.live.observe_last_result = false;
                    i += 1;
                    continue;
                }
                "--behavioral" => {
                    opts.live.behavioral = true;
                    i += 1;
                    continue;
                }
                "--param-domain" => {
                    let v = os_arg_to_str(value(s)?, "--param-domain value")?;
                    let (tree, index, domain) =
                        parse_param_domain_arg(v).map_err(|e| e.to_string())?;
                    assumed.push((tree, index, domain, format!("--param-domain {v}")));
                    i += 2;
                    continue;
                }
                "--v2-enum-domains" => {
                    enum_sources.push(PathBuf::from(value(s)?));
                    i += 2;
                    continue;
                }
                "--pack" => {
                    pack = Some(PathBuf::from(value(s)?));
                    i += 2;
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
    if pack.is_some() && enum_sources.is_empty() {
        return Err("--pack only applies to --v2-enum-domains".to_owned().into());
    }
    for source in &enum_sources {
        let shown = sanitize_path_display(source);
        let found = v2_enum_param_domains(source, pack.as_deref())
            .map_err(|e| format!("--v2-enum-domains {shown}: {e}"))?;
        if found.is_empty() {
            eprintln!("note: --v2-enum-domains {shown}: no tree takes an enum-typed parameter");
        }
        for d in found {
            assumed.push((
                d.tree,
                d.param_index,
                d.domain,
                format!(
                    "{shown}: tree {} parameter {}: {}",
                    d.tree_name, d.param_name, d.enum_name
                ),
            ));
        }
    }
    let mut out = OutWriter::stdout();
    if !assumed.is_empty() {
        out.writeln_str(
            "ASSUMING parameter domains: an EQUIVALENT verdict holds only for runs that enter \
             each constrained tree with its parameters inside these sets",
        )?;
    }
    for (tree, index, domain, from) in assumed {
        let slot = opts.param_domains.entry(tree).or_default().entry(index);
        let line = match slot {
            std::collections::btree_map::Entry::Occupied(e) if *e.get() != domain => {
                return Err(format!(
                    "BHAV #{tree} param[{index}] is assumed both in {{{}}} and in {{{domain}}} ({from})",
                    e.get()
                )
                .into());
            }
            std::collections::btree_map::Entry::Occupied(_) => continue,
            std::collections::btree_map::Entry::Vacant(e) => {
                let line = format!("ASSUME: BHAV #{tree} param[{index}] in {{{domain}}} ({from})");
                e.insert(domain);
                line
            }
        };
        out.writeln_str(&sanitize_single_line(&line))?;
    }
    let read = |path: &Path| -> Result<Vec<u8>, CliError> {
        let (mut file, len, hdr) = read_regular_file_prefix(path)?;
        Ok(read_bounded_iff_file(path, &mut file, len, hdr)?)
    };
    // Each library supplies only its own range (`program.rs:1015`): the global file
    // trees 256 to 4095, a semi-global file 8192 and up, with the argument counts the
    // VM's loader keeps. The files are trusted to be the libraries the game loads; two
    // that disagree on a tree cannot both be, so that is an error.
    //
    // A tree is argument-free (its call reads no operand bytes) only if every file
    // that supplies it says so, and its count is 0.
    let mut library: std::collections::BTreeMap<u16, (Option<usize>, bool, PathBuf)> =
        std::collections::BTreeMap::new();
    for (extra, semi) in &with {
        let bytes = read(extra)?;
        let arities =
            library_arities_from_iff(&bytes).map_err(|e| format!("{}: {e}", extra.display()))?;
        let argless = library_argless_from_iff(&bytes, *semi)
            .map_err(|e| format!("{}: {e}", extra.display()))?;
        let in_range = |id: u16| {
            if *semi {
                id >= 8192
            } else {
                (256..=4095).contains(&id)
            }
        };
        for (id, argc) in arities.into_iter().filter(|&(id, _)| in_range(id)) {
            let mut is_argless = argless.contains(&id);
            if let Some((seen, seen_argless, from)) = library.get(&id) {
                if *seen != argc {
                    let count = |n: &Option<usize>| {
                        n.map_or_else(
                            || "an unparseable tree".to_owned(),
                            |n| format!("{n} arguments"),
                        )
                    };
                    return Err(format!(
                        "library tree #{id} has {} in {} but {} in {}",
                        count(seen),
                        from.display(),
                        count(&argc),
                        extra.display()
                    )
                    .into());
                }
                is_argless &= *seen_argless;
            }
            library.insert(id, (argc, is_argless, extra.clone()));
        }
    }
    opts.extra_argless.extend(
        library
            .iter()
            .filter(|(_, (argc, argless, _))| *argless && *argc == Some(0))
            .map(|(&id, _)| id),
    );
    opts.extra_arities.extend(
        library
            .into_iter()
            .filter_map(|(id, (argc, _, _))| Some((id, argc?))),
    );
    let (a, b) = (read(&path_a)?, read(&path_b)?);
    let report = check_iff_equivalent(&a, &b, &opts).map_err(|e| e.to_string())?;

    let mut counts = [0usize; 4];
    for line in &report.assumptions {
        out.writeln_str(&sanitize_single_line(&format!("ASSUME: {line}")))?;
    }
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
        "{word}: {} resources ({} identical, {} equivalent, {} different, {} inconclusive){}",
        report.resources.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        if verdict == IffVerdict::Equivalent {
            BUDGET_ASSUMPTION
        } else {
            ""
        }
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
        let resolved = FileArities::private_only(&iff, "test");
        let res = emit_lifted_bhavs(
            &iff,
            &chunks,
            "test.iff",
            None,
            false,
            Some(&resolved),
            &std::collections::BTreeMap::new(),
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
