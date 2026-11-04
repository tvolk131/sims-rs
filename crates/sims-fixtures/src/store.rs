//! Immutable local blob storage. Exact decoded bytes are content identity.
//! This is a trusted-local filesystem format, not a hostile-filesystem sandbox.
//! Symlinks and unexpected layout entries are rejected when inspected. No GC
//! deletion API exists: callers receive a reachability/size preview only.
use crate::{CaseManifest, model};
use anyhow::{Context, Result, bail, ensure};
use flate2::{Compression as GzipLevel, bufread::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 8] = b"SIMSBL01";
const HEADER_BYTES: usize = 49;
const MAX_BLOB_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_VALUE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct BlobRef {
    pub sha256: String,
    pub bytes: u64,
}
impl BlobRef {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid lowercase SHA-256 identity"
        );
        ensure!(
            self.bytes <= MAX_BLOB_BYTES,
            "blob exceeds storage payload limit"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Raw,
    Gzip,
}
impl Compression {
    fn byte(self) -> u8 {
        match self {
            Self::Raw => 0,
            Self::Gzip => 1,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    format: String,
    version: u32,
}

/// Explicit retained research roots, separate from immutable reference cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub schema: u32,
    pub id: String,
    pub reason: String,
    pub blobs: Vec<BlobRef>,
}
/// Compact execution evidence. Storage does not interpret an outcome as parity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReport {
    pub schema: u32,
    pub id: String,
    pub case_id: String,
    pub outcome: String,
    pub notes: Vec<String>,
    pub artifacts: Vec<BlobRef>,
}
#[derive(Debug, Default, Serialize)]
pub struct VerificationReport {
    pub cases: usize,
    pub pins: usize,
    pub runs: usize,
    pub blobs: usize,
    pub payload_bytes: u64,
    pub stored_bytes: u64,
}
#[derive(Debug, Serialize)]
pub struct GcCandidate {
    pub blob: BlobRef,
    pub stored_bytes: u64,
}
#[derive(Debug, Serialize)]
pub struct GcPreview {
    pub dry_run: bool,
    pub referenced_blobs: usize,
    pub unreferenced: Vec<GcCandidate>,
    pub unreferenced_stored_bytes: u64,
    /// Opaque payload dependency declarations are attested by the producer;
    /// Recognized trace/archive inventory dependencies are independently checked.
    pub dependency_policy: String,
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}
impl Store {
    /// Initialize a new or empty directory; never adopt an unrelated tree.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.try_exists()? {
            directory(path)?;
            ensure!(
                fs::read_dir(path)?.next().is_none(),
                "store initialization requires an empty directory"
            );
        } else {
            create_directory_chain(path)?;
            directory(path)?;
        }
        let root = path.canonicalize()?;
        for name in ["blobs", "blobs/sha256", "cases", "pins", "runs", "tmp"] {
            let path = root.join(name);
            fs::create_dir(&path)?;
            sync_directory(path.parent().unwrap())?;
        }
        let store = Self { root };
        store.publish_json(
            &store.root.join("store.json"),
            &Marker {
                format: "sims-fixtures".into(),
                version: 1,
            },
        )?;
        Self::open(store.root)
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        directory(path.as_ref())?;
        let store = Self {
            root: path.as_ref().canonicalize()?,
        };
        store.layout()?;
        Ok(store)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn layout(&self) -> Result<()> {
        directory(&self.root)?;
        let marker: Marker = read_json(&self.root.join("store.json"), 4096)?;
        ensure!(
            marker.format == "sims-fixtures" && marker.version == 1,
            "unsupported fixture store format/version"
        );
        for name in ["blobs", "blobs/sha256", "cases", "pins", "runs", "tmp"] {
            directory(&self.root.join(name))?;
        }
        Ok(())
    }
    pub fn blob_path(&self, blob: &BlobRef) -> Result<PathBuf> {
        blob.validate()?;
        self.layout()?;
        Ok(self
            .root
            .join("blobs/sha256")
            .join(&blob.sha256[..2])
            .join(format!("{}.blob", &blob.sha256[2..])))
    }
    fn checked_blob_path(&self, blob: &BlobRef) -> Result<PathBuf> {
        let path = self.blob_path(blob)?;
        directory(path.parent().unwrap())?;
        regular(&path)?;
        Ok(path)
    }
    /// Stream an exact payload into a temporary object and publish only after
    /// serialization/compression, hash/length and durable file writes succeed.
    pub fn put_with(
        &self,
        compression: Compression,
        write: impl FnOnce(&mut dyn Write) -> Result<()>,
    ) -> Result<BlobRef> {
        self.layout()?;
        let (temp, mut file) = TempFile::new(&self.root.join("tmp"))?;
        file.write_all(&[0; HEADER_BYTES])?;
        let sink = match compression {
            Compression::Raw => Sink::Raw(file),
            Compression::Gzip => Sink::Gzip(GzEncoder::new(file, GzipLevel::fast())),
        };
        let mut writer = PayloadWriter {
            sink,
            hash: Sha256::new(),
            bytes: 0,
        };
        write(&mut writer)?;
        let bytes = writer.bytes;
        let hash: [u8; 32] = writer.hash.finalize().into();
        let mut file = writer.sink.finish()?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(MAGIC)?;
        file.write_all(&[compression.byte()])?;
        file.write_all(&bytes.to_le_bytes())?;
        file.write_all(&hash)?;
        file.sync_all()?;
        drop(file);
        let blob = BlobRef {
            sha256: hex(&hash),
            bytes,
        };
        let path = self.blob_path(&blob)?;
        ensure_directory(path.parent().unwrap())?;
        match fs::hard_link(&temp.path, &path) {
            Ok(()) => sync_directory(path.parent().unwrap())?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // A filename is not proof that an earlier object is intact.
                self.verify_blob(&blob)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(blob)
    }
    pub fn put_reader(&self, mut reader: impl Read, compression: Compression) -> Result<BlobRef> {
        self.put_with(compression, |writer| {
            io::copy(&mut reader, writer)?;
            Ok(())
        })
    }
    pub fn put_file(&self, path: impl AsRef<Path>, compression: Compression) -> Result<BlobRef> {
        let file = File::open(path.as_ref())?;
        ensure!(
            file.metadata()?.is_file(),
            "fixture source is not a regular file"
        );
        self.put_reader(BufReader::new(file), compression)
    }
    pub fn put_bytes(&self, bytes: &[u8], compression: Compression) -> Result<BlobRef> {
        self.put_reader(bytes, compression)
    }
    /// Named struct maps preserve missing/optional fields across schema changes.
    /// Byte-heavy DTOs must explicitly use serde_bytes for packed binary buffers.
    pub fn put_value<T: Serialize>(&self, value: &T) -> Result<BlobRef> {
        self.put_with(Compression::Gzip, |writer| {
            // MessagePack emits small writes for ordinary byte vectors. Batch
            // those before hashing/compression without changing decoded bytes.
            let mut output = BufWriter::with_capacity(64 << 10, writer);
            value.serialize(&mut rmp_serde::Serializer::new(&mut output).with_struct_map())?;
            // Flush the buffer without adding an underlying gzip sync flush.
            output.into_inner().map_err(|error| error.into_error())?;
            Ok(())
        })
    }
    pub fn get_value<T: DeserializeOwned>(&self, blob: &BlobRef) -> Result<T> {
        let bytes = self.read_bytes(blob, MAX_VALUE_BYTES)?;
        let mut decoder = rmp_serde::Deserializer::new(io::Cursor::new(&bytes));
        let value = T::deserialize(&mut decoder)?;
        ensure!(
            decoder.position() == bytes.len() as u64,
            "trailing MessagePack payload"
        );
        Ok(value)
    }
    /// The destination can receive bytes before a late checksum failure; callers
    /// publishing derived files must use their own temporary/atomic writer.
    pub fn copy_blob(&self, blob: &BlobRef, output: &mut dyn Write) -> Result<()> {
        let path = self.checked_blob_path(blob)?;
        let mut file = BufReader::new(File::open(path)?);
        let (found, compression) = read_header(&mut file)?;
        ensure!(
            found == *blob,
            "blob header identity/length differs from reference"
        );
        let mut hash = Sha256::new();
        let mut count = 0u64;
        match compression {
            Compression::Raw => copy_checked(&mut file, output, blob.bytes, &mut count, &mut hash)?,
            Compression::Gzip => {
                let mut decoder = GzDecoder::new(file);
                copy_checked(&mut decoder, output, blob.bytes, &mut count, &mut hash)?;
                let mut remaining = decoder.into_inner();
                ensure!(
                    remaining.fill_buf()?.is_empty(),
                    "trailing compressed data/member"
                );
            }
        }
        ensure!(
            count == blob.bytes,
            "decoded blob length differs from reference"
        );
        ensure!(
            hex(&hash.finalize()) == blob.sha256,
            "decoded blob SHA-256 mismatch"
        );
        Ok(())
    }
    pub fn read_bytes(&self, blob: &BlobRef, limit: u64) -> Result<Vec<u8>> {
        blob.validate()?;
        ensure!(blob.bytes <= limit, "blob exceeds requested read limit");
        let mut result = Vec::new();
        // Avoid allocating attacker-declared sizes before checking payloads.
        self.copy_blob(blob, &mut result)?;
        Ok(result)
    }
    pub fn verify_blob(&self, blob: &BlobRef) -> Result<()> {
        self.copy_blob(blob, &mut io::sink())
    }

    pub fn publish_case(&self, case: &CaseManifest) -> Result<()> {
        self.validate_case(case)?;
        let directory = self.root.join("cases").join(&case.id);
        ensure_directory(&directory)?;
        self.publish_json(&directory.join("manifest.json"), case)
    }
    pub fn load_case(&self, id: &str) -> Result<CaseManifest> {
        self.layout()?;
        model::validate_id(id)?;
        let directory_path = self.root.join("cases").join(id);
        directory(&directory_path)?;
        let case: CaseManifest =
            read_json(&directory_path.join("manifest.json"), MAX_MANIFEST_BYTES)?;
        ensure!(case.id == id, "case directory and manifest identity differ");
        self.validate_case(&case)?;
        Ok(case)
    }
    pub fn list_cases(&self) -> Result<Vec<String>> {
        self.layout()?;
        let mut ids = Vec::new();
        for entry in fs::read_dir(self.root.join("cases"))? {
            let entry = entry?;
            directory(&entry.path())?;
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 case ID"))?;
            model::validate_id(&id)?;
            // A crashed pre-publication writer may leave an empty directory.
            if entry.path().join("manifest.json").try_exists()? {
                ids.push(id);
            } else {
                ensure!(
                    fs::read_dir(entry.path())?.next().is_none(),
                    "unexpected unpublished case contents"
                );
            }
        }
        ids.sort();
        Ok(ids)
    }
    fn validate_case(&self, case: &CaseManifest) -> Result<()> {
        self.layout()?;
        case.validate()?;
        for blob in case.blob_refs() {
            self.verify_blob(&blob)?;
        }
        for artifact in case.all_artifacts() {
            if artifact.format == model::ArtifactFormat::TraceIndex {
                ensure!(
                    artifact.schema == crate::trace::TRACE_INDEX_SCHEMA,
                    "unknown TraceIndex artifact schema"
                );
                let index: crate::trace::TraceIndex = self.get_value(&artifact.blob)?;
                index.verify(self)?;
                let expected = references(index.blob_refs())?;
                let declared = references(artifact.dependencies.clone())?;
                ensure!(
                    expected == declared,
                    "TraceIndex dependency closure differs from declaration"
                );
            } else if artifact.format == model::ArtifactFormat::ArchiveInventory {
                ensure!(
                    artifact.schema == crate::archive::ARCHIVE_SCHEMA,
                    "unknown ArchiveInventory artifact schema"
                );
                ensure!(
                    artifact.blob.bytes <= crate::archive::MAX_INVENTORY_BYTES,
                    "ArchiveInventory exceeds read limit"
                );
                let inventory: crate::archive::ArchiveInventory = self.get_value(&artifact.blob)?;
                inventory.validate()?;
                let expected = references(inventory.blob_refs())?;
                let declared = references(artifact.dependencies.clone())?;
                ensure!(
                    expected == declared,
                    "ArchiveInventory dependency closure differs from declaration"
                );
            }
        }
        Ok(())
    }
    pub fn pin(&self, pin: &Pin) -> Result<()> {
        validate_pin(pin)?;
        for blob in &pin.blobs {
            self.verify_blob(blob)?;
        }
        self.publish_json(
            &self.root.join("pins").join(format!("{}.json", pin.id)),
            pin,
        )
    }
    pub fn record_run(&self, report: &RunReport) -> Result<()> {
        validate_run(report)?;
        self.load_case(&report.case_id)?;
        for blob in &report.artifacts {
            self.verify_blob(blob)?;
        }
        self.publish_json(
            &self.root.join("runs").join(format!("{}.json", report.id)),
            report,
        )
    }
    /// Export a selected case and its verified declared closure. No paths to the
    /// source store or hardlinked source blobs are required by the new bundle.
    pub fn export_case(&self, id: &str, destination: impl AsRef<Path>) -> Result<()> {
        let case = self.load_case(id)?;
        let destination = destination.as_ref();
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        directory(parent)?;
        ensure!(
            !parent.canonicalize()?.starts_with(&self.root),
            "export must be outside the source store"
        );
        // Reserve the destination with create_dir, whose already-exists failure
        // is atomic. Until the final store.json link, Store::open rejects it.
        // This avoids rename-over-empty-directory races between ordinary users.
        let mut reservation = TempDirectory::reserve(destination)?;
        let stage = TempDirectory::new(&reservation.path)?;
        let target = Self::create(&stage.path)?;
        for blob in case.blob_refs() {
            let copied =
                target.put_with(Compression::Gzip, |writer| self.copy_blob(&blob, writer))?;
            ensure!(copied == blob, "export changed blob identity");
        }
        target.publish_case(&case)?;
        target.verify_all()?;
        for name in ["blobs", "cases", "pins", "runs", "tmp"] {
            fs::rename(stage.path.join(name), reservation.path.join(name))?;
        }
        fs::hard_link(
            stage.path.join("store.json"),
            reservation.path.join("store.json"),
        )?;
        reservation.persist = true;
        sync_directory(&reservation.path)?;
        sync_directory(parent)?;
        Ok(())
    }
    pub fn verify_all(&self) -> Result<VerificationReport> {
        let (_, mut report) = self.roots()?;
        for blob in self.all_blobs()? {
            self.verify_blob(&blob)?;
            report.blobs += 1;
            report.payload_bytes = report
                .payload_bytes
                .checked_add(blob.bytes)
                .context("payload count overflow")?;
            report.stored_bytes += fs::metadata(self.checked_blob_path(&blob)?)?.len();
        }
        Ok(report)
    }
    pub fn gc_preview(&self) -> Result<GcPreview> {
        let (roots, _) = self.roots()?;
        let mut unreferenced = Vec::new();
        let mut total = 0;
        for blob in self.all_blobs()? {
            self.verify_blob(&blob)?;
            if !roots.contains_key(&blob.sha256) {
                let stored_bytes = fs::metadata(self.checked_blob_path(&blob)?)?.len();
                total += stored_bytes;
                unreferenced.push(GcCandidate { blob, stored_bytes });
            }
        }
        Ok(GcPreview { dry_run: true, referenced_blobs: roots.len(), unreferenced,
            unreferenced_stored_bytes: total,
            dependency_policy: "Declared flattened references; trace/archive inventory closures verified; other payloads producer-attested. Preview only, no deletion.".into() })
    }
    fn roots(&self) -> Result<(BTreeMap<String, BlobRef>, VerificationReport)> {
        self.layout()?;
        let mut refs = Vec::new();
        let mut report = VerificationReport::default();
        for id in self.list_cases()? {
            refs.extend(self.load_case(&id)?.blob_refs());
            report.cases += 1;
        }
        for path in json_entries(&self.root.join("pins"))? {
            let pin: Pin = read_json(&path, MAX_MANIFEST_BYTES)?;
            validate_pin(&pin)?;
            ensure!(
                path.file_stem().and_then(|p| p.to_str()) == Some(&pin.id),
                "pin filename identity differs"
            );
            refs.extend(pin.blobs);
            report.pins += 1;
        }
        for path in json_entries(&self.root.join("runs"))? {
            let run: RunReport = read_json(&path, MAX_MANIFEST_BYTES)?;
            validate_run(&run)?;
            ensure!(
                path.file_stem().and_then(|p| p.to_str()) == Some(&run.id),
                "run filename identity differs"
            );
            self.load_case(&run.case_id)?;
            refs.extend(run.artifacts);
            report.runs += 1;
        }
        let refs = references(refs)?;
        for blob in refs.values() {
            self.verify_blob(blob)?;
        }
        Ok((refs, report))
    }
    fn all_blobs(&self) -> Result<Vec<BlobRef>> {
        self.layout()?;
        let mut result = Vec::new();
        for shard in fs::read_dir(self.root.join("blobs/sha256"))? {
            let shard = shard?;
            directory(&shard.path())?;
            let prefix = shard
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 shard"))?;
            ensure!(
                prefix.len() == 2
                    && prefix
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid blob shard"
            );
            for file in fs::read_dir(shard.path())? {
                let path = file?.path();
                regular(&path)?;
                let suffix = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .context("non-UTF8 blob name")?
                    .strip_suffix(".blob")
                    .context("unexpected blob filename")?;
                let mut file = File::open(&path)?;
                let (blob, _) = read_header(&mut file)?;
                ensure!(
                    blob.sha256 == format!("{prefix}{suffix}"),
                    "blob pathname differs from header identity"
                );
                blob.validate()?;
                result.push(blob);
            }
        }
        result.sort();
        Ok(result)
    }
    fn publish_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        directory(path.parent().context("manifest parent absent")?)?;
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        ensure!(
            bytes.len() as u64 <= MAX_MANIFEST_BYTES,
            "manifest exceeds storage size limit"
        );
        let (temp, mut file) = TempFile::new(&self.root.join("tmp"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        match fs::hard_link(&temp.path, path) {
            Ok(()) => sync_directory(path.parent().unwrap())?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                regular(path)?;
                ensure!(
                    fs::metadata(path)?.len() == bytes.len() as u64 && fs::read(path)? == bytes,
                    "immutable manifest already exists with different contents: {}",
                    path.display()
                );
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

fn validate_pin(pin: &Pin) -> Result<()> {
    ensure!(pin.schema == 1, "unsupported pin schema");
    model::validate_id(&pin.id)?;
    ensure!(!pin.reason.trim().is_empty(), "pin reason absent");
    ensure!(pin.blobs.len() <= 100_000, "pin too large");
    references(pin.blobs.clone())?;
    Ok(())
}
fn validate_run(run: &RunReport) -> Result<()> {
    ensure!(run.schema == 1, "unsupported run report schema");
    model::validate_id(&run.id)?;
    model::validate_id(&run.case_id)?;
    ensure!(!run.outcome.trim().is_empty(), "run outcome absent");
    ensure!(
        run.artifacts.len() <= 100_000,
        "run artifact list too large"
    );
    references(run.artifacts.clone())?;
    Ok(())
}
fn references(refs: Vec<BlobRef>) -> Result<BTreeMap<String, BlobRef>> {
    let mut result = BTreeMap::new();
    for reference in refs {
        reference.validate()?;
        if let Some(previous) = result.insert(reference.sha256.clone(), reference.clone()) {
            ensure!(
                previous == reference,
                "contradictory blob lengths for same digest"
            );
        }
    }
    Ok(result)
}
fn directory(path: &Path) -> Result<()> {
    ensure!(
        fs::symlink_metadata(path)
            .with_context(|| format!("directory {}", path.display()))?
            .file_type()
            .is_dir(),
        "expected non-symlink directory: {}",
        path.display()
    );
    Ok(())
}
fn ensure_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => sync_directory(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    directory(path)
}
fn create_directory_chain(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => return directory(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    create_directory_chain(parent)?;
    ensure_directory(path)
}
fn regular(path: &Path) -> Result<()> {
    ensure!(
        fs::symlink_metadata(path)
            .with_context(|| format!("file {}", path.display()))?
            .file_type()
            .is_file(),
        "expected non-symlink regular file: {}",
        path.display()
    );
    Ok(())
}
fn read_json<T: DeserializeOwned>(path: &Path, max: u64) -> Result<T> {
    regular(path)?;
    let mut file = File::open(path)?;
    ensure!(file.metadata()?.len() <= max, "JSON metadata too large");
    let mut raw = Vec::new();
    (&mut file).take(max + 1).read_to_end(&mut raw)?;
    ensure!(
        raw.len() as u64 <= max,
        "JSON metadata grew beyond size limit"
    );
    Ok(serde_json::from_slice(&raw)?)
}
fn json_entries(path: &Path) -> Result<Vec<PathBuf>> {
    directory(path)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        regular(&path)?;
        ensure!(
            path.extension().is_some_and(|s| s == "json"),
            "unexpected metadata entry"
        );
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}
fn read_header(reader: &mut impl Read) -> Result<(BlobRef, Compression)> {
    let mut header = [0; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    ensure!(&header[..8] == MAGIC, "unsupported blob magic/version");
    let compression = match header[8] {
        0 => Compression::Raw,
        1 => Compression::Gzip,
        _ => bail!("unsupported blob compression"),
    };
    let blob = BlobRef {
        sha256: hex(&header[17..49]),
        bytes: u64::from_le_bytes(header[9..17].try_into().unwrap()),
    };
    blob.validate()?;
    Ok((blob, compression))
}
fn copy_checked(
    reader: &mut impl Read,
    out: &mut dyn Write,
    expected: u64,
    count: &mut u64,
    hash: &mut Sha256,
) -> Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        *count = count
            .checked_add(n as u64)
            .context("payload size overflow")?;
        ensure!(*count <= expected, "decoded blob exceeds declared size");
        hash.update(&buffer[..n]);
        out.write_all(&buffer[..n])?;
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    Ok(())
}
fn token() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    )
}
struct TempFile {
    path: PathBuf,
}
impl TempFile {
    fn new(parent: &Path) -> Result<(Self, File)> {
        directory(parent)?;
        let path = parent.join(format!("{}.partial", token()));
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        Ok((Self { path }, file))
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
struct TempDirectory {
    path: PathBuf,
    persist: bool,
}
impl TempDirectory {
    fn new(parent: &Path) -> Result<Self> {
        let path = parent.join(format!(".sims-fixtures-export-{}", token()));
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            persist: false,
        })
    }
    fn reserve(path: &Path) -> Result<Self> {
        fs::create_dir(path).context("export destination must not already exist")?;
        Ok(Self {
            path: path.to_path_buf(),
            persist: false,
        })
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        if !self.persist {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
enum Sink {
    Raw(File),
    Gzip(GzEncoder<File>),
}
impl Sink {
    fn finish(self) -> Result<File> {
        Ok(match self {
            Self::Raw(file) => file,
            Self::Gzip(gzip) => gzip.finish()?,
        })
    }
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Raw(f) => f.write(bytes),
            Self::Gzip(f) => f.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Raw(f) => f.flush(),
            Self::Gzip(f) => f.flush(),
        }
    }
}
struct PayloadWriter {
    sink: Sink,
    hash: Sha256,
    bytes: u64,
}
impl Write for PayloadWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .checked_add(bytes.len() as u64)
            .is_none_or(|n| n > MAX_BLOB_BYTES)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "blob payload limit exceeded",
            ));
        }
        let n = self.sink.write(bytes)?;
        self.bytes += n as u64;
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest;
    use serde_json::json;

    #[test]
    fn codec_independent_identity_and_dedup() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store::create(temp.path().join("store"))?;
        let raw = store.put_bytes(b"same original bytes\n", Compression::Raw)?;
        let compressed = store.put_bytes(b"same original bytes\n", Compression::Gzip)?;
        assert_eq!(raw, compressed);
        assert_eq!(raw.sha256, digest(b"same original bytes\n"));
        assert_eq!(store.verify_all()?.blobs, 1);
        assert_eq!(store.read_bytes(&raw, 100)?, b"same original bytes\n");
        Ok(())
    }
    #[test]
    fn failed_stream_is_unpublished_and_cleans_temporary() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store::create(temp.path().join("store"))?;
        let result = store.put_with(Compression::Gzip, |writer| {
            writer.write_all(b"uncommitted partial bytes")?;
            bail!("authored producer failure")
        });
        assert!(result.is_err());
        assert_eq!(store.verify_all()?.blobs, 0);
        assert_eq!(fs::read_dir(store.root().join("tmp"))?.count(), 0);
        Ok(())
    }
    #[test]
    fn named_binary_roundtrip_preserves_omissions_bytes_and_float_bits() -> Result<()> {
        #[derive(Debug, Serialize, Deserialize)]
        struct Data {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            absent: Option<u32>,
            after: u64,
            #[serde(with = "serde_bytes")]
            bytes: Vec<u8>,
            single: f32,
            double: f64,
        }
        let temp = tempfile::tempdir()?;
        let store = Store::create(temp.path().join("store"))?;
        let input = Data {
            absent: None,
            after: u64::MAX,
            bytes: (0..=255).collect(),
            single: f32::from_bits(0xffc12345),
            double: f64::from_bits(0x8000000000000000),
        };
        let reference = store.put_value(&input)?;
        let output: Data = store.get_value(&reference)?;
        assert_eq!(output.absent, None);
        assert_eq!(output.after, input.after);
        assert_eq!(output.bytes, input.bytes);
        assert_eq!(output.single.to_bits(), input.single.to_bits());
        assert_eq!(output.double.to_bits(), input.double.to_bits());
        let unknown = store.put_value(&json!({"case_schema":999}))?;
        assert!(store.get_value::<CaseManifest>(&unknown).is_err());
        Ok(())
    }
    #[test]
    fn buffered_values_keep_canonical_named_map_bytes_across_buffer_edges() -> Result<()> {
        let input = BTreeMap::from([
            (
                7u16,
                (0..131_089).map(|i| (i % 256) as u8).collect::<Vec<_>>(),
            ),
            (1024u16, vec![0; 65_537]),
        ]);
        let expected = rmp_serde::to_vec_named(&input)?;
        let temp = tempfile::tempdir()?;
        let store = Store::create(temp.path().join("store"))?;
        let blob = store.put_value(&input)?;
        assert_eq!(blob.bytes, expected.len() as u64);
        assert_eq!(blob.sha256, digest(&expected));
        assert_eq!(store.read_bytes(&blob, 1 << 20)?, expected);
        assert_eq!(store.get_value::<BTreeMap<u16, Vec<u8>>>(&blob)?, input);
        Ok(())
    }
    #[test]
    fn serialization_failure_after_buffer_flushes_is_unpublished() -> Result<()> {
        struct Failing;
        impl Serialize for Failing {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::{Error, SerializeSeq};
                let mut sequence = serializer.serialize_seq(Some(131_089))?;
                for _ in 0..131_088 {
                    sequence.serialize_element(&255u8)?;
                }
                Err(S::Error::custom(
                    "authored serialization failure after buffered writes",
                ))
            }
        }
        let temp = tempfile::tempdir()?;
        let store = Store::create(temp.path().join("store"))?;
        assert!(store.put_value(&Failing).is_err());
        assert_eq!(store.verify_all()?.blobs, 0);
        assert_eq!(fs::read_dir(store.root().join("tmp"))?.count(), 0);
        Ok(())
    }
    #[test]
    fn pins_keep_selected_evidence_and_gc_is_read_only() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store::create(temp.path().join("store"))?;
        let pinned = store.put_bytes(b"authoritative", Compression::Gzip)?;
        let orphan = store.put_bytes(b"uncommitted run", Compression::Gzip)?;
        store.pin(&Pin {
            schema: 1,
            id: "research".into(),
            reason: "keep actual evidence".into(),
            blobs: vec![pinned],
        })?;
        let before = store.verify_all()?.stored_bytes;
        let preview = store.gc_preview()?;
        assert!(preview.dry_run);
        assert_eq!(preview.unreferenced.len(), 1);
        assert_eq!(preview.unreferenced[0].blob, orphan);
        assert_eq!(store.verify_all()?.stored_bytes, before);
        Ok(())
    }
}
