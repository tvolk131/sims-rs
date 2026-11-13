//! Owned PE32 bytes observed from one bounded descriptor read.
//!
//! The image and its borrowed views describe immutable, preferred-base logical
//! **initial-file** bytes. They do not describe a running Windows module,
//! resolved imports, relocations, TLS/CRT initialization, unpacking, execution,
//! or constructor-entry globals. In particular, an initial zero is not proof of
//! a constructor-entry value.
//! [`entry`] separately owns a mutable logical copy for its checked entry-image
//! prefix, without qualifying CPU/stack, imported calls or later initialization.
//!
//! Descriptor metadata equality is a read-consistency guard. It does not defend
//! against a malicious same-UID rewrite-and-restore, reserve a pathname, or make
//! an inode immutable. The final path component cannot be a symlink; atomicity
//! of parent-directory lookup or pathname replacement is not claimed.

use anyhow::Result;
use sims_formats::pe32;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

pub mod entry;
pub mod imports;

/// Metadata obtained from the opened descriptor, never from a separate path stat.
/// Copies are observations and cannot authorize a module or an initial view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorMetadata {
    pub device: u64,
    pub inode: u64,
    pub file_size: u64,
    /// Full Unix mode, including the file type and permission bits.
    pub mode: u32,
    pub link_count: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
    pub changed_seconds: i64,
    pub changed_nanoseconds: i64,
    pub user_id: u32,
    pub group_id: u32,
}

/// The successful read retained alongside its exact parsed bytes.
///
/// `supplied_path` preserves the caller's OS-string spelling without
/// canonicalization. It is an observation, not a continuing path binding.
/// `sha256` hashes exactly `bytes_read` bytes consumed by the PE parser. These
/// values, including copies of this struct, confer no module lifetime identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub supplied_path: PathBuf,
    pub sha256: [u8; 32],
    pub file_size: u64,
    pub bytes_read: u64,
    pub before: DescriptorMetadata,
    pub after: DescriptorMetadata,
}

struct OriginIdentity;

/// Opaque, process-local identity of one successful load's lifetime.
///
/// Cloning retains that identity. Equality uses shared allocation identity, not
/// bytes, digest, logical address, path, descriptor metadata or an integer token.
/// Retaining a token after its Module is dropped cannot manufacture a new view
/// or attach that identity to a later load.
///
/// The identity has no public constructor:
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::ModuleOrigin;
/// let _ = ModuleOrigin(std::sync::Arc::new(()));
/// ```
#[derive(Clone)]
pub struct ModuleOrigin(Arc<OriginIdentity>);

impl PartialEq for ModuleOrigin {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ModuleOrigin {}

impl fmt::Debug for ModuleOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ModuleOrigin(..)")
    }
}

/// One immutable parsed initial-file image and the descriptor read it came from.
/// Only [`read`] can create a Module. Equal bytes in separate loads do not imply
/// equal module lifetimes. This type has no byte adoption or mutable access API.
pub struct Module {
    observation: Observation,
    image: pe32::Image,
    origin: ModuleOrigin,
}

impl fmt::Debug for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("observation", &self.observation)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl Module {
    pub fn observation(&self) -> &Observation {
        &self.observation
    }

    pub fn image(&self) -> &pe32::Image {
        &self.image
    }

    pub fn origin(&self) -> ModuleOrigin {
        self.origin.clone()
    }

    /// Consume this exact read into the checked module-image entry prefix.
    /// A refusal returns the same Module; this does not execute its CPU caller,
    /// imports, protection driver, CRT or constructors.
    pub fn begin_entry(self) -> std::result::Result<entry::Entry, entry::BeginError> {
        entry::Entry::begin(self)
    }

    /// Consume the actual read into a normal-import preflight. No binding,
    /// image store, source advance, CPU caller or driver execution occurs.
    pub fn begin_initial_imports(self) -> std::result::Result<imports::ImportPlan, imports::BeginError> {
        imports::ImportPlan::begin(self)
    }

    /// Borrow a bounded preferred-base initial-file span from this load.
    /// Query validation is the parsed image's `initial_va` policy, including
    /// rejection of empty, overflowing, unmapped and crossing ranges.
    ///
    /// A view cannot outlive its Module:
    /// ```compile_fail
    /// use sims_content::native_bootstrap::executable::read;
    /// use std::path::Path;
    /// let span = {
    ///     let module = read(Path::new("authored.exe")).unwrap();
    ///     module.initial_span(0x0040_1000, 1).unwrap()
    /// };
    /// let _ = span.bytes();
    /// ```
    pub fn initial_span(&self, va: u32, len: usize) -> Result<InitialSpan<'_>> {
        let bytes = self.image.initial_va(va, len)?;
        Ok(InitialSpan {
            module: self,
            address: va,
            bytes,
        })
    }
}

/// An immutable initial-file view borrowed from one Module, never from a token.
///
/// Bytes exposed by a view cannot be written:
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::Module;
/// fn overwrite(module: &Module) {
///     let span = module.initial_span(0x0040_1000, 1).unwrap();
///     span.bytes()[0] = 7;
/// }
/// ```
pub struct InitialSpan<'a> {
    module: &'a Module,
    address: u32,
    bytes: &'a [u8],
}

impl InitialSpan<'_> {
    pub fn address(&self) -> u32 {
        self.address
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub fn origin(&self) -> ModuleOrigin {
        self.module.origin()
    }
}

impl fmt::Debug for InitialSpan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitialSpan")
            .field("address", &self.address)
            .field("length", &self.bytes.len())
            .field("origin", &self.module.origin)
            .finish_non_exhaustive()
    }
}

/// Read and parse one regular file through exactly one open descriptor.
///
/// On macOS/Linux the descriptor is opened with `O_NOFOLLOW`, `O_NONBLOCK` and
/// `O_CLOEXEC`. Its type and bounded length are checked before reading, and its
/// identity, size, mode, link count, timestamps and ownership must agree after
/// reading. At most `pe32::MAX_FILE_BYTES + 1` bytes are read. The same owned Vec
/// is hashed and consumed by the parser; every failure returns no Module.
///
/// The descriptor closes after the observation is made. The immutable bytes,
/// observation and per-load origin remain together for the Module's lifetime.
/// Other operating systems fail closed without opening the path.
pub fn read(path: &Path) -> Result<Module> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        descriptor::read_with_hook(path, |_, _| Ok(()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        anyhow::bail!("secure initial-file descriptor reads require macOS or Linux")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod descriptor {
    use super::*;
    use anyhow::{Context, ensure};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{File, OpenOptions},
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };

    // A private deterministic test seam. Hooks cannot supply bytes, descriptors,
    // origins or images; production uses the no-op hook in `read`.
    #[derive(Clone, Copy, Debug)]
    pub(super) enum ReadPhase {
        BeforeRead,
        AfterRead {
            // Only test hooks inspect this count; production discards phases.
            #[cfg_attr(not(test), allow(dead_code))]
            bytes_read: usize,
        },
    }

    fn metadata(file: &File) -> Result<DescriptorMetadata> {
        let m = file
            .metadata()
            .context("stat opened initial-file descriptor")?;
        ensure!(m.is_file(), "initial-file descriptor is not a regular file");
        Ok(DescriptorMetadata {
            device: m.dev(),
            inode: m.ino(),
            file_size: m.len(),
            mode: m.mode(),
            link_count: m.nlink(),
            modified_seconds: m.mtime(),
            modified_nanoseconds: m.mtime_nsec(),
            changed_seconds: m.ctime(),
            changed_nanoseconds: m.ctime_nsec(),
            user_id: m.uid(),
            group_id: m.gid(),
        })
    }

    pub(super) fn validate_read(
        before: &DescriptorMetadata,
        after: &DescriptorMetadata,
        bytes_read: usize,
    ) -> Result<()> {
        ensure!(
            bytes_read <= pe32::MAX_FILE_BYTES,
            "initial-file read exceeds byte bound"
        );
        ensure!(
            before == after,
            "initial-file descriptor metadata changed during read"
        );
        ensure!(
            u64::try_from(bytes_read)? == before.file_size,
            "initial-file read length differs from descriptor length"
        );
        Ok(())
    }

    pub(super) fn read_with_hook(
        path: &Path,
        mut hook: impl FnMut(ReadPhase, &File) -> Result<()>,
    ) -> Result<Module> {
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .with_context(|| format!("open initial-file descriptor {}", path.display()))?;
        let before = metadata(&file)?;
        ensure!(
            before.file_size <= u64::try_from(pe32::MAX_FILE_BYTES)?,
            "initial-file descriptor length exceeds byte bound"
        );
        let limit = pe32::MAX_FILE_BYTES
            .checked_add(1)
            .context("initial-file read bound overflow")?;
        let mut bytes = Vec::new();
        // Reserve the entire bounded read fallibly so read_to_end cannot need
        // an unchecked growth allocation if the file grows after its first stat.
        bytes
            .try_reserve_exact(limit)
            .context("reserve bounded initial-file bytes")?;
        hook(ReadPhase::BeforeRead, &file)?;
        (&mut file)
            .take(u64::try_from(limit)?)
            .read_to_end(&mut bytes)
            .context("read bounded initial-file descriptor")?;
        hook(
            ReadPhase::AfterRead {
                bytes_read: bytes.len(),
            },
            &file,
        )?;
        let after = metadata(&file)?;
        validate_read(&before, &after, bytes.len())?;
        let sha256 = Sha256::digest(&bytes).into();
        let observation = Observation {
            supplied_path: path.to_path_buf(),
            sha256,
            file_size: before.file_size,
            bytes_read: u64::try_from(bytes.len())?,
            before,
            after,
        };
        let image = pe32::Image::parse(bytes).context("parse bounded PE32 initial-file image")?;
        Ok(Module {
            observation,
            image,
            origin: ModuleOrigin(Arc::new(OriginIdentity)),
        })
    }
}

#[cfg(test)]
mod tests;
