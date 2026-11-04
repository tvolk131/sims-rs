//! Byte-exact archives of explicitly selected regular files. No source deletion,
//! parser/World normalization, implicit recursive selection, or resume claim.
//! Restores are independent bundles: `files/<original-relative-path>`, an exact
//! inventory, and a completion marker published last. Without that marker an
//! interrupted destination is NOT a usable restored archive.
use crate::{Artifact, ArtifactFormat, BlobRef, CaseManifest, Compression, DependencyMode, Store};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const ARCHIVE_SCHEMA: u32 = 1;
pub const MAX_ARCHIVE_FILES: usize = 10_000;
pub const MAX_INVENTORY_BYTES: u64 = 16 * 1024 * 1024;
pub const COMPLETION_FILE: &str = "archive-complete.json";
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024;
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OriginalMetadata {
    /// Signed decimal seconds.nanoseconds, audit-only; never silently restored.
    pub modified_unix: Option<String>,
    pub readonly: bool,
    /// Audit-only original Unix permission/special bits. No uid/gid is stored.
    pub unix_mode: Option<u32>,
    /// Original filesystem alias identity, audit-only; restore materializes
    /// independent files instead of reproducing source hardlink topology.
    pub unix_device: Option<u64>,
    pub unix_inode: Option<u64>,
    pub unix_links: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveFile {
    pub relative_path: String,
    /// Hash and size of the exact original file bytes, before compression.
    pub blob: BlobRef,
    pub metadata: OriginalMetadata,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveInventory {
    pub schema: u32,
    /// Inert logical provenance, never reopened during restore.
    pub source_locator: Option<String>,
    pub files: Vec<ArchiveFile>,
    pub total_bytes: u64,
}
impl ArchiveInventory {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == ARCHIVE_SCHEMA,
            "unsupported archive inventory schema"
        );
        ensure!(
            !self.files.is_empty() && self.files.len() <= MAX_ARCHIVE_FILES,
            "invalid archive file count"
        );
        if let Some(source) = &self.source_locator {
            ensure!(
                !source.trim().is_empty() && source.len() <= 65_536 && !source.contains('\0'),
                "invalid archive source locator"
            );
        }
        validate_paths(self.files.iter().map(|f| f.relative_path.as_str()))?;
        let mut total = 0u64;
        let mut identities = BTreeMap::new();
        for file in &self.files {
            file.blob.validate()?;
            ensure!(
                file.blob.bytes <= MAX_FILE_BYTES,
                "archive file exceeds byte limit"
            );
            total = total
                .checked_add(file.blob.bytes)
                .context("archive size overflow")?;
            if let Some(previous) = identities.insert(&file.blob.sha256, file.blob.bytes) {
                ensure!(
                    previous == file.blob.bytes,
                    "same archive digest has conflicting lengths"
                );
            }
            if let Some(modified) = &file.metadata.modified_unix {
                let unsigned = modified.strip_prefix('-').unwrap_or(modified);
                let Some((seconds, nanos)) = unsigned.split_once('.') else {
                    bail!("invalid original modified time");
                };
                ensure!(
                    !seconds.is_empty()
                        && seconds.len() <= 20
                        && seconds.bytes().all(|b| b.is_ascii_digit())
                        && nanos.len() == 9
                        && nanos.bytes().all(|b| b.is_ascii_digit()),
                    "invalid original modified time"
                );
            }
            ensure!(
                matches!(
                    (
                        file.metadata.unix_device,
                        file.metadata.unix_inode,
                        file.metadata.unix_links
                    ),
                    (None, None, None) | (Some(_), Some(_), Some(1..))
                ),
                "incomplete/invalid Unix alias metadata"
            );
            if let Some(mode) = file.metadata.unix_mode {
                ensure!(
                    mode <= 0o7777 && file.metadata.readonly == (mode & 0o222 == 0),
                    "invalid original permission metadata"
                );
            }
        }
        ensure!(
            total == self.total_bytes && total <= MAX_TOTAL_BYTES,
            "archive total byte count differs or exceeds limit"
        );
        Ok(())
    }
    pub fn blob_refs(&self) -> Vec<BlobRef> {
        let mut refs: Vec<_> = self.files.iter().map(|f| f.blob.clone()).collect();
        refs.sort();
        refs.dedup();
        refs
    }
    pub fn validate_artifact(&self, artifact: &Artifact) -> Result<()> {
        self.validate()?;
        artifact.validate()?;
        ensure!(
            artifact.format == ArtifactFormat::ArchiveInventory
                && artifact.schema == ARCHIVE_SCHEMA
                && artifact.dependency_mode == DependencyMode::Declared,
            "archive artifact schema/dependency mode differs"
        );
        let mut declared = artifact.dependencies.clone();
        declared.sort();
        declared.dedup();
        ensure!(
            declared == self.blob_refs(),
            "archive declared dependency closure differs"
        );
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ArchiveOptions {
    pub id: String,
    pub title: String,
    pub source_locator: Option<String>,
    pub compression: Compression,
    pub commands: Vec<Vec<String>>,
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveReport {
    pub case_id: String,
    pub inventory: BlobRef,
    pub files: usize,
    pub original_bytes: u64,
    pub qualification: &'static str,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub schema: u32,
    pub case_id: String,
    pub inventory: BlobRef,
    pub files: usize,
    pub total_bytes: u64,
    pub payload_root: String,
    pub metadata_policy: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub case_id: String,
    pub destination: PathBuf,
    pub payload_root: PathBuf,
    pub files: usize,
    pub original_bytes: u64,
    pub inventory: BlobRef,
}

fn relative(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 4096
            && !value.starts_with('/')
            && !value.chars().any(|c| c < ' ' || "\\:<>\"|?*".contains(c)),
        "archive path must be a portable relative file name"
    );
    for part in value.split('/') {
        ensure!(
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' ']),
            "archive path has empty/dot/parent or trailing-dot/space components"
        );
        let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "reserved device archive path"
        );
    }
    ensure!(
        Path::new(value).is_relative(),
        "absolute archive path rejected"
    );
    Ok(())
}
fn validate_paths<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let mut folded = BTreeSet::new();
    let mut spellings = BTreeMap::new();
    for value in values {
        relative(value)?;
        for end in value
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(value.len()))
        {
            let actual = &value[..end];
            if let Some(previous) =
                spellings.insert(actual.to_ascii_lowercase(), actual.to_string())
            {
                ensure!(
                    previous == actual,
                    "inconsistent ASCII spelling of archive path component"
                );
            }
        }
        ensure!(
            folded.insert(value.to_ascii_lowercase()),
            "duplicate or ASCII case-alias archive path"
        );
    }
    // Adjacent sorted names are insufficient: a-b sorts between a and a/child.
    for value in &folded {
        for (offset, _) in value.match_indices('/') {
            ensure!(
                !folded.contains(&value[..offset]),
                "overlapping file/ancestor archive paths"
            );
        }
    }
    Ok(())
}
fn directory(path: &Path) -> Result<Metadata> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("inspect directory {}", path.display()))?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "directory is absent, special or a symlink: {}",
        path.display()
    );
    Ok(meta)
}
fn regular(path: &Path) -> Result<Metadata> {
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("inspect regular file {}", path.display()))?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "archive input is not a regular non-symlink file: {}",
        path.display()
    );
    Ok(meta)
}
fn contained_file(root: &Path, name: &str) -> Result<(PathBuf, Metadata)> {
    relative(name)?;
    let mut path = root.to_path_buf();
    let parts: Vec<_> = name.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        path.push(part);
        directory(&path)?;
    }
    path.push(parts[parts.len() - 1]);
    let meta = regular(&path)?;
    Ok((path, meta))
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    len: u64,
    modified: Option<SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64, u32),
}
impl Snapshot {
    fn new(meta: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            readonly: meta.permissions().readonly(),
            #[cfg(unix)]
            identity: (
                meta.dev(),
                meta.ino(),
                meta.ctime(),
                meta.ctime_nsec(),
                meta.mode(),
            ),
        }
    }
}
fn same_directory(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (a.dev(), a.ino()) == (b.dev(), b.ino())
    }
    #[cfg(not(unix))]
    {
        a.created().ok() == b.created().ok()
    }
}
fn original_metadata(meta: &Metadata) -> OriginalMetadata {
    let modified_unix = meta
        .modified()
        .ok()
        .map(|t| match t.duration_since(UNIX_EPOCH) {
            Ok(d) => format!("{}.{:09}", d.as_secs(), d.subsec_nanos()),
            Err(e) => {
                let d = e.duration();
                format!("-{}.{:09}", d.as_secs(), d.subsec_nanos())
            }
        });
    #[cfg(unix)]
    let unix_mode = {
        use std::os::unix::fs::MetadataExt;
        Some(meta.mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let unix_mode = None;
    #[cfg(unix)]
    let (unix_device, unix_inode, unix_links) = {
        use std::os::unix::fs::MetadataExt;
        (Some(meta.dev()), Some(meta.ino()), Some(meta.nlink()))
    };
    #[cfg(not(unix))]
    let (unix_device, unix_inode, unix_links) = (None, None, None);
    OriginalMetadata {
        modified_unix,
        readonly: meta.permissions().readonly(),
        unix_mode,
        unix_device,
        unix_inode,
        unix_links,
    }
}
fn hash_file(file: &mut File, reference: &BlobRef) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut length = 0u64;
    let mut buffer = [0u8; 65_536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(read as u64)
            .context("archive verification length overflow")?;
        ensure!(
            length <= reference.bytes,
            "source grew during archive/restore verification"
        );
        hash.update(&buffer[..read]);
    }
    ensure!(
        length == reference.bytes && format!("{:x}", hash.finalize()) == reference.sha256,
        "archive file bytes changed or digest differs"
    );
    Ok(())
}

pub fn archive_files(
    store: &Store,
    source_root: &Path,
    files: &[String],
    options: &ArchiveOptions,
) -> Result<ArchiveReport> {
    archive_files_inner(store, source_root, files, options, &mut |_| Ok(()))
}
fn archive_files_inner(
    store: &Store,
    source_root: &Path,
    files: &[String],
    options: &ArchiveOptions,
    after_blob: &mut impl FnMut(&Path) -> Result<()>,
) -> Result<ArchiveReport> {
    crate::model::validate_id(&options.id)?;
    ensure!(
        !options.title.trim().is_empty() && options.title.len() <= 1024,
        "invalid archive title"
    );
    ensure!(
        !files.is_empty() && files.len() <= MAX_ARCHIVE_FILES,
        "invalid explicit archive file count"
    );
    validate_paths(files.iter().map(String::as_str))?;
    let root_meta = directory(source_root)?;
    let root = source_root.canonicalize()?;
    let mut snapshots = Vec::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for name in files {
        let (path, before) = contained_file(&root, name)?;
        ensure!(
            !path.starts_with(store.root()),
            "archive cannot include its own store"
        );
        ensure!(
            before.len() <= MAX_FILE_BYTES,
            "selected file exceeds archive byte limit"
        );
        total = total
            .checked_add(before.len())
            .context("archive total size overflow")?;
        ensure!(
            total <= MAX_TOTAL_BYTES,
            "explicit archive exceeds total byte limit"
        );
        let snapshot = Snapshot::new(&before);
        let mut input = File::open(&path)?;
        ensure!(
            snapshot == Snapshot::new(&input.metadata()?),
            "source identity changed before archive read"
        );
        let blob = store.put_reader((&mut input).take(before.len() + 1), options.compression)?;
        ensure!(
            blob.bytes == before.len(),
            "source size changed during archive read"
        );
        after_blob(&path)?;
        hash_file(&mut input, &blob)?;
        ensure!(
            snapshot == Snapshot::new(&input.metadata()?),
            "source metadata changed during archive read"
        );
        let (_, after) = contained_file(&root, name)?;
        ensure!(
            snapshot == Snapshot::new(&after),
            "source path/metadata changed during archive read"
        );
        entries.push(ArchiveFile {
            relative_path: name.clone(),
            blob,
            metadata: original_metadata(&before),
        });
        snapshots.push((name.clone(), snapshot));
    }
    ensure!(
        same_directory(&root_meta, &directory(source_root)?),
        "archive source root identity changed"
    );
    for (name, before) in snapshots {
        let (_, after) = contained_file(&root, &name)?;
        ensure!(
            before == Snapshot::new(&after),
            "source changed before archive case publication"
        );
    }
    let inventory = ArchiveInventory {
        schema: ARCHIVE_SCHEMA,
        source_locator: options.source_locator.clone(),
        files: entries,
        total_bytes: total,
    };
    inventory.validate()?;
    let encoded = rmp_serde::to_vec_named(&inventory)?;
    ensure!(
        encoded.len() as u64 <= MAX_INVENTORY_BYTES,
        "archive inventory exceeds limit"
    );
    let blob = store.put_bytes(&encoded, Compression::Gzip)?;
    let artifact = Artifact::declared(
        blob.clone(),
        ArtifactFormat::ArchiveInventory,
        ARCHIVE_SCHEMA,
        inventory.blob_refs(),
    );
    let mut notes = options.notes.clone();
    notes.push("Byte-exact selected-file archive only; no simulation, resumability, semantic or parity qualification".into());
    let mut case = CaseManifest::minimal_import(&options.id, &options.title, artifact, notes);
    case.provenance.source_locator = options.source_locator.clone();
    case.provenance.commands = options.commands.clone();
    store.publish_case(&case)?;
    Ok(ArchiveReport {
        case_id: options.id.clone(),
        inventory: blob,
        files: inventory.files.len(),
        original_bytes: total,
        qualification: "byte-exact selected files; no resume/parity claim",
    })
}

struct Stage {
    path: PathBuf,
}
impl Stage {
    fn new(parent: &Path) -> Result<Self> {
        for _ in 0..100 {
            let name = format!(
                ".sims-archive-stage-{}-{}",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
            );
            let path = parent.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        bail!("could not reserve owned archive staging directory")
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
fn create_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("create without overwrite {}", path.display()))
}
fn parents(root: &Path, name: &str, owned: &mut BTreeSet<PathBuf>) -> Result<()> {
    let mut path = root.to_path_buf();
    let parts: Vec<_> = name.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        path.push(part);
        if !owned.contains(&path) {
            fs::create_dir(&path)
                .with_context(|| format!("reserve new restore directory {}", path.display()))?;
            owned.insert(path.clone());
        }
        directory(&path)?;
    }
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn require_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(_) => bail!("restore destination already exists: {}", path.display()),
    }
}
fn load_inventory(store: &Store, id: &str) -> Result<(ArchiveInventory, BlobRef)> {
    let case = store.load_case(id)?;
    ensure!(
        case.source.format == ArtifactFormat::ArchiveInventory
            && case.source.schema == ARCHIVE_SCHEMA,
        "case is not a supported archive"
    );
    ensure!(
        case.source.blob.bytes <= MAX_INVENTORY_BYTES,
        "archive inventory exceeds read limit"
    );
    let inventory: ArchiveInventory = store.get_value(&case.source.blob)?;
    inventory.validate_artifact(&case.source)?;
    Ok((inventory, case.source.blob))
}

pub fn restore_archive(store: &Store, id: &str, destination: &Path) -> Result<RestoreReport> {
    restore_inner(store, id, destination, &mut |_| Ok(()))
}
fn restore_inner(
    store: &Store,
    id: &str,
    destination: &Path,
    after_staging: &mut impl FnMut(&Path) -> Result<()>,
) -> Result<RestoreReport> {
    let (inventory, reference) = load_inventory(store, id)?;
    require_absent(destination)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    directory(parent)?;
    let parent = parent.canonicalize()?;
    ensure!(
        !parent.starts_with(store.root()),
        "restore destination must be outside source store"
    );
    let name = destination
        .file_name()
        .context("restore destination needs a file name")?;
    let destination = parent.join(name);
    let stage = Stage::new(&parent)?;
    let staged_files = stage.path.join("files");
    fs::create_dir(&staged_files)?;
    let mut owned = BTreeSet::from([staged_files.clone()]);
    for entry in &inventory.files {
        parents(&staged_files, &entry.relative_path, &mut owned)?;
        let path = staged_files.join(&entry.relative_path);
        let mut output = create_file(&path)?;
        store.copy_blob(&entry.blob, &mut output)?;
        output.sync_all()?;
    }
    let mut inventory_file = create_file(&stage.path.join("inventory.msgpack"))?;
    store.copy_blob(&reference, &mut inventory_file)?;
    inventory_file.sync_all()?;
    drop(inventory_file);
    after_staging(&stage.path)?;
    for entry in &inventory.files {
        let (path, _) = contained_file(&staged_files, &entry.relative_path)?;
        hash_file(&mut File::open(path)?, &entry.blob)?;
    }
    let completion=Completion{schema:ARCHIVE_SCHEMA,case_id:id.into(),inventory:reference.clone(),files:inventory.files.len(),total_bytes:inventory.total_bytes,
        payload_root:"files".into(),metadata_policy:"Original mtime/mode/readonly retained for audit only; restored files use private/default permissions. No uid/gid, privileged mode or mtime restoration.".into()};
    let mut marker = create_file(&stage.path.join(COMPLETION_FILE))?;
    serde_json::to_writer_pretty(&mut marker, &completion)?;
    marker.write_all(b"\n")?;
    marker.sync_all()?;
    drop(marker);
    // Prove the complete staged bundle, including inventory and unexpected paths,
    // before reserving any final destination.
    verify_restored(&stage.path)?;
    // Reserve the destination atomically. File publication uses create-only
    // links to NEW materialized staging inodes, never store/source blob inodes.
    fs::create_dir(&destination)?;
    let payload = destination.join("files");
    fs::create_dir(&payload)?;
    let mut published = BTreeSet::from([payload.clone()]);
    for entry in &inventory.files {
        parents(&payload, &entry.relative_path, &mut published)?;
        fs::hard_link(
            staged_files.join(&entry.relative_path),
            payload.join(&entry.relative_path),
        )?;
    }
    fs::hard_link(
        stage.path.join("inventory.msgpack"),
        destination.join("inventory.msgpack"),
    )?;
    // Persist each populated directory bottom-up before committing the marker.
    let mut directories: Vec<_> = published.into_iter().collect();
    directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in directories {
        sync_directory(&dir)?;
    }
    sync_directory(&destination)?;
    fs::hard_link(
        stage.path.join(COMPLETION_FILE),
        destination.join(COMPLETION_FILE),
    )?;
    sync_directory(&destination)?;
    sync_directory(&parent)?;
    verify_restored(&destination)
}

/// Verify an independently restored bundle without needing the original store,
/// source directory, original absolute locator or source file permissions.
pub fn verify_restored(destination: &Path) -> Result<RestoreReport> {
    directory(destination)?;
    let marker_path = destination.join(COMPLETION_FILE);
    regular(&marker_path)?;
    let mut raw = Vec::new();
    File::open(marker_path)?
        .take(1_048_577)
        .read_to_end(&mut raw)?;
    ensure!(
        raw.len() <= 1_048_576,
        "archive completion marker exceeds limit"
    );
    let completion: Completion = serde_json::from_slice(&raw)?;
    ensure!(
        completion.schema == ARCHIVE_SCHEMA && completion.payload_root == "files",
        "unsupported restored archive marker"
    );
    crate::model::validate_id(&completion.case_id)?;
    completion.inventory.validate()?;
    ensure!(
        completion.inventory.bytes <= MAX_INVENTORY_BYTES,
        "restored inventory exceeds limit"
    );
    let inventory_path = destination.join("inventory.msgpack");
    regular(&inventory_path)?;
    let mut file = File::open(&inventory_path)?;
    hash_file(&mut file, &completion.inventory)?;
    file.seek(SeekFrom::Start(0))?;
    let mut raw = Vec::new();
    file.take(MAX_INVENTORY_BYTES + 1).read_to_end(&mut raw)?;
    let mut decoder = rmp_serde::Deserializer::new(std::io::Cursor::new(&raw));
    let inventory = ArchiveInventory::deserialize(&mut decoder)?;
    ensure!(
        decoder.position() == raw.len() as u64,
        "trailing restored inventory data"
    );
    inventory.validate()?;
    ensure!(
        completion.files == inventory.files.len()
            && completion.total_bytes == inventory.total_bytes,
        "restored archive totals differ"
    );
    let payload = destination.join("files");
    directory(&payload)?;
    let mut expected_files = BTreeSet::new();
    let mut expected_dirs = BTreeSet::from([payload.clone()]);
    for entry in &inventory.files {
        let (path, _) = contained_file(&payload, &entry.relative_path)?;
        hash_file(&mut File::open(&path)?, &entry.blob)?;
        expected_files.insert(path.clone());
        let mut parent = path.parent();
        while let Some(p) = parent {
            if !p.starts_with(&payload) {
                break;
            }
            expected_dirs.insert(p.to_path_buf());
            if p == payload {
                break;
            }
            parent = p.parent();
        }
    }
    let mut stack = vec![payload.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let meta = fs::symlink_metadata(&path)?;
            ensure!(
                !meta.file_type().is_symlink(),
                "symlink in restored payload"
            );
            if meta.is_dir() {
                ensure!(
                    expected_dirs.contains(&path),
                    "unexpected restored directory"
                );
                stack.push(path);
            } else {
                ensure!(
                    meta.is_file() && expected_files.contains(&path),
                    "unexpected restored file"
                );
            }
        }
    }
    let expected = BTreeSet::from([
        "files".to_string(),
        "inventory.msgpack".to_string(),
        COMPLETION_FILE.to_string(),
    ]);
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        ensure!(
            expected.contains(
                entry
                    .file_name()
                    .to_str()
                    .context("nonUTF8 restored entry")?
            ),
            "unexpected restored bundle entry"
        );
    }
    Ok(RestoreReport {
        case_id: completion.case_id,
        destination: destination.to_path_buf(),
        payload_root: payload,
        files: inventory.files.len(),
        original_bytes: inventory.total_bytes,
        inventory: completion.inventory,
    })
}

#[cfg(test)]
mod tests;
