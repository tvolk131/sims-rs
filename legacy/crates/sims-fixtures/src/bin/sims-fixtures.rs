//! Small explicit fixture-store commands. `gc` is preview-only; there is no
//! deletion command, implicit source discovery, or simulator dependency.
use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::Value;
use sims_fixtures::{
    Artifact, ArtifactFormat, BlobRef, CaseManifest, Compression, Store,
    archive::{self, ArchiveOptions},
    legacy::{self, ImportOptions, JsonlOptions},
    trace::TraceOptions,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, BufReader, Read, Write},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    name = "sims-fixtures",
    about = "Versioned research fixtures; integrity is not gameplay parity"
)]
struct Cli {
    #[arg(long, global = true, help = "Explicit fixture-store directory")]
    store: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Create a store in a new or empty directory.
    Init,
    /// Stream exact stdin bytes directly into a compressed unqualified source case.
    Capture {
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        source_id: Option<String>,
        #[arg(long, default_value_t = 16 * 1024 * 1024 * 1024)]
        max_bytes: u64,
        #[arg(long = "note")]
        notes: Vec<String>,
    },
    /// Stream a verified case source, requiring its expected byte identity.
    Source {
        id: String,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        bytes: u64,
    },
    /// Retain exact JSON bytes plus a normalized MessagePack observation.
    ImportJson {
        input: PathBuf,
        #[command(flatten)]
        options: ImportArgs,
        #[arg(long, default_value_t = 64 * 1024 * 1024)]
        max_input_bytes: u64,
    },
    /// Stream a bounded raw JSONL prefix plus reconstructable observations.
    ImportJsonl {
        input: PathBuf,
        #[command(flatten)]
        options: ImportArgs,
        #[arg(long, default_value_t = legacy::DEFAULT_RECORD_LIMIT)]
        record_limit: u64,
        #[arg(long, default_value_t = legacy::MAX_RECORDS)]
        max_records: u64,
        #[arg(long, default_value_t = legacy::DEFAULT_MAX_LINE_BYTES)]
        max_line_bytes: u64,
        #[arg(long, default_value_t = legacy::DEFAULT_MAX_PREFIX_BYTES)]
        max_prefix_bytes: u64,
        #[arg(long, default_value_t = legacy::DEFAULT_MAX_TRACE_RECORD_BYTES,
              help = "Maximum normalized MessagePack record bytes, independent of raw JSON line size")]
        max_trace_record_bytes: u64,
        #[arg(long, default_value_t = legacy::DEFAULT_MAX_TRACE_CHUNK_BYTES)]
        max_trace_chunk_bytes: u64,
        #[arg(
            long,
            help = "Optional source JSON pointer; only actual unsigned integers are accepted"
        )]
        tick_pointer: Option<String>,
        #[arg(
            long,
            help = "Optional source JSON pointer; true/false is recorded without certifying a completed tick"
        )]
        advanced_pointer: Option<String>,
    },
    /// Archive an explicit JSON list of relative regular-file paths, byte-for-byte.
    Archive {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(
            long,
            help = "JSON array of explicit portable relative paths; no recursive selection"
        )]
        files_from: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, help = "Inert source locator; never reopened by restore")]
        source_id: Option<String>,
        #[arg(long, value_enum, default_value_t = Codec::Gzip)]
        compression: Codec,
        #[arg(long = "note")]
        notes: Vec<String>,
    },
    /// Materialize an archive into a NEW destination/files/<relative-path> bundle.
    Restore { id: String, destination: PathBuf },
    /// Independently verify a completed restored bundle; no store is required.
    VerifyRestored { destination: PathBuf },
    /// Verify every declared case/blob and recognized trace/archive reconstruction.
    Verify,
    /// Show a case manifest, or list case IDs when no ID is supplied.
    Inspect { id: Option<String> },
    /// Export one case and its complete declared closure into a new directory.
    Export { id: String, destination: PathBuf },
    /// Preview unreachable blobs. --dry-run is mandatory; nothing is deleted.
    Gc {
        #[arg(long, required = true)]
        dry_run: bool,
    },
    /// Copy exact decoded blob bytes for read/debug use. Files are create-only.
    Get {
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        bytes: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Codec {
    Raw,
    Gzip,
}
impl From<Codec> for Compression {
    fn from(value: Codec) -> Self {
        match value {
            Codec::Raw => Self::Raw,
            Codec::Gzip => Self::Gzip,
        }
    }
}
#[derive(Args)]
struct ImportArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long, value_enum, default_value_t = Codec::Gzip)]
    compression: Codec,
    #[arg(long, help = "Inert logical source ID; defaults to the input basename")]
    source_id: Option<String>,
    #[arg(
        long,
        help = "Caller-declared SHA256 of the complete original source, not the selected prefix"
    )]
    origin_sha256: Option<String>,
    #[arg(
        long,
        help = "Caller-declared source-image SHA256; importer never opens a source image"
    )]
    source_image_sha256: Option<String>,
    #[arg(
        long,
        help = "Bounded JSON provenance declaration; not independent verification"
    )]
    provenance: Option<PathBuf>,
    #[arg(
        long,
        help = "Bounded JSON coverage declaration; import remains unqualified"
    )]
    coverage: Option<PathBuf>,
    #[arg(long = "note")]
    notes: Vec<String>,
}
impl ImportArgs {
    fn prepare(self, input: &std::path::Path) -> Result<ImportOptions> {
        let source_locator = Some(match self.source_id {
            Some(id) => id,
            None => legacy::logical_source(input)?,
        });
        Ok(ImportOptions {
            title: self.title.unwrap_or_else(|| self.id.clone()),
            id: self.id,
            compression: self.compression.into(),
            source_locator,
            origin_sha256: self.origin_sha256,
            source_image_sha256: self.source_image_sha256,
            provenance: declaration(self.provenance)?,
            coverage: declaration(self.coverage)?,
            notes: self.notes,
            commands: vec![invocation()?],
        })
    }
}
fn invocation() -> Result<Vec<String>> {
    invocation_from(std::env::args_os())
}
fn invocation_from(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Vec<String>> {
    arguments
        .into_iter()
        .enumerate()
        .map(|(index, argument)| {
            argument.into_string().map_err(|_| {
                anyhow::anyhow!("cannot record non-UTF-8 invocation argument {index} without loss")
            })
        })
        .collect()
}

#[cfg(all(test, unix))]
mod invocation_tests {
    use super::invocation_from;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    #[test]
    fn conversion_preserves_arguments_or_rejects_non_utf8_without_loss() {
        assert_eq!(
            invocation_from([
                OsString::from("sims-fixtures"),
                OsString::from("one argument with spaces")
            ])
            .unwrap(),
            ["sims-fixtures", "one argument with spaces"]
        );
        let error = invocation_from([
            OsString::from("sims-fixtures"),
            OsString::from_vec(b"unrepresentable-\xff".to_vec()),
        ])
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "cannot record non-UTF-8 invocation argument 1 without loss"
        );
    }
}
fn declaration(path: Option<PathBuf>) -> Result<Option<Value>> {
    path.map(|path| {
        let mut raw = vec![];
        File::open(&path)
            .with_context(|| format!("open declaration {}", path.display()))?
            .take(65_001)
            .read_to_end(&mut raw)?;
        ensure!(raw.len() <= 65_000, "declaration exceeds 65000 bytes");
        serde_json::from_slice(&raw)
            .with_context(|| format!("parse declaration {}", path.display()))
    })
    .transpose()
}
fn print_json(value: &impl Serialize) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Command::VerifyRestored { destination } = &cli.command {
        return print_json(&archive::verify_restored(destination)?);
    }
    let store_path = cli
        .store
        .context("--store must name an explicit fixture-store directory")?;
    if matches!(cli.command, Command::Init) {
        let store = Store::create(&store_path)?;
        return print_json(&serde_json::json!({"initialized":store.root()}));
    }
    let store = Store::open(&store_path)?;
    match cli.command {
        Command::Init => unreachable!("handled without opening a preexisting store"),
        Command::Capture {
            id,
            title,
            source_id,
            max_bytes,
            mut notes,
        } => {
            ensure!(
                max_bytes > 0 && max_bytes <= 64 * 1024 * 1024 * 1024,
                "capture limit must be between 1 byte and 64 GiB"
            );
            let mut case = CaseManifest::minimal_import(
                id.clone(),
                title.unwrap_or_else(|| id.clone()),
                Artifact::self_contained(
                    BlobRef {
                        sha256: sims_fixtures::digest(b""),
                        bytes: 0,
                    },
                    ArtifactFormat::Raw,
                    1,
                ),
                vec![],
            );
            notes.push("Exact stdin bytes only. EOF does not certify producer success, a completed game tick, an input schedule, or resumability.".into());
            case.notes = notes;
            case.provenance.source_locator = source_id;
            case.provenance.commands = vec![invocation()?];
            case.validate()?;
            ensure!(
                !store.list_cases()?.contains(&id),
                "capture case already exists"
            );
            case.source.blob = store.put_with(Compression::Gzip, |writer| {
                {
                    let mut out = io::stdout().lock();
                    serde_json::to_writer(
                        &mut out,
                        &serde_json::json!({"ready":true,"case_id":id}),
                    )?;
                    out.write_all(b"\n")?;
                    out.flush()?;
                }
                let copied = io::copy(&mut io::stdin().lock().take(max_bytes + 1), writer)?;
                ensure!(copied <= max_bytes, "capture exceeds byte limit");
                Ok(())
            })?;
            store.publish_case(&case)?;
            print_json(
                &serde_json::json!({"case_id":case.id,"source":case.source.blob,
                                         "qualification":"unqualified_stdin_capture"}),
            )
        }
        Command::Source { id, sha256, bytes } => {
            let expected = BlobRef { sha256, bytes };
            expected.validate()?;
            let case = store.load_case(&id)?;
            ensure!(case.source.blob == expected, "case source identity differs");
            store.copy_blob(&expected, &mut io::stdout().lock())
        }
        Command::ImportJson {
            input,
            options,
            max_input_bytes,
        } => {
            let options = options.prepare(&input)?;
            let reader =
                File::open(&input).with_context(|| format!("open source {}", input.display()))?;
            print_json(&legacy::import_json(
                &store,
                reader,
                &options,
                max_input_bytes,
            )?)
        }
        Command::ImportJsonl {
            input,
            options,
            record_limit,
            max_records,
            max_line_bytes,
            max_prefix_bytes,
            max_trace_record_bytes,
            max_trace_chunk_bytes,
            tick_pointer,
            advanced_pointer,
        } => {
            let options = options.prepare(&input)?;
            let bounds = JsonlOptions {
                record_limit,
                max_records,
                max_line_bytes,
                max_prefix_bytes,
                tick_pointer,
                advanced_pointer,
                trace_options: TraceOptions {
                    max_record_bytes: max_trace_record_bytes,
                    max_chunk_bytes: max_trace_chunk_bytes,
                    ..TraceOptions::default()
                },
            };
            let mut reader = BufReader::new(
                File::open(&input).with_context(|| format!("open source {}", input.display()))?,
            );
            print_json(&legacy::import_jsonl(
                &store,
                &mut reader,
                &options,
                &bounds,
            )?)
        }
        Command::Archive {
            source_root,
            files_from,
            id,
            title,
            source_id,
            compression,
            notes,
        } => {
            const MAX_PATH_LIST_BYTES: u64 = 64 * 1024 * 1024;
            let mut raw = Vec::new();
            File::open(&files_from)
                .with_context(|| {
                    format!("open explicit archive path list {}", files_from.display())
                })?
                .take(MAX_PATH_LIST_BYTES + 1)
                .read_to_end(&mut raw)?;
            ensure!(
                raw.len() as u64 <= MAX_PATH_LIST_BYTES,
                "archive path list exceeds limit"
            );
            let files: Vec<String> = serde_json::from_slice(&raw)
                .context("archive path list must be a JSON array of relative strings")?;
            let options = ArchiveOptions {
                title: title.unwrap_or_else(|| id.clone()),
                id,
                source_locator: source_id,
                compression: compression.into(),
                commands: vec![invocation()?],
                notes,
            };
            print_json(&archive::archive_files(
                &store,
                &source_root,
                &files,
                &options,
            )?)
        }
        Command::Restore { id, destination } => {
            print_json(&archive::restore_archive(&store, &id, &destination)?)
        }
        Command::VerifyRestored { .. } => unreachable!("handled without a store"),
        Command::Verify => print_json(&store.verify_all()?),
        Command::Inspect { id } => match id {
            Some(id) => print_json(&store.load_case(&id)?),
            None => print_json(&store.list_cases()?),
        },
        Command::Export { id, destination } => {
            store.export_case(&id, &destination)?;
            print_json(&serde_json::json!({"exported_case":id,"destination":destination}))
        }
        Command::Gc { dry_run } => {
            ensure!(dry_run, "only gc --dry-run is implemented");
            print_json(&store.gc_preview()?)
        }
        Command::Get {
            sha256,
            bytes,
            output,
        } => {
            let reference = BlobRef { sha256, bytes };
            reference.validate()?;
            // Detect corrupt/missing content before creating a named output.
            // copy_blob independently validates again while copying.
            store.verify_blob(&reference)?;
            if let Some(path) = output {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .with_context(|| {
                        format!("create output without overwrite {}", path.display())
                    })?;
                store.copy_blob(&reference, &mut file).with_context(|| {
                    format!(
                        "copy failed; owned output may be partial: {}",
                        path.display()
                    )
                })?;
                file.sync_all()?;
            } else {
                store.copy_blob(&reference, &mut io::stdout().lock())?;
            }
            Ok(())
        }
    }
}
