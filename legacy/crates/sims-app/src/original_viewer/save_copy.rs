//! Alternate output policy only. Capture/schema and atomic_save remain Save's.
use anyhow::{Context, Result, ensure};
use std::{path::{Path, PathBuf}, sync::Arc};

#[derive(Debug, Clone)]
pub(super) struct Owner(Arc<()>);
impl Owner {
    pub(super) fn new() -> Self { Self(Arc::new(())) }
    pub(super) fn same(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}

pub(super) fn seed(canonical: &Path) -> PathBuf {
    let mut stem = canonical.file_stem().unwrap_or_default().to_os_string();
    stem.push(" copy");
    let mut path = canonical.with_file_name(stem);
    if let Some(extension) = canonical.extension() { path.set_extension(extension); }
    path
}

// Resolve existing parents even if the leaf or several directories do not yet
// exist. Do not create anything, reopen/adopt an Input or claim a pathname lock.
fn resolved(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() { path.to_owned() }
        else { std::env::current_dir()?.join(path) };
    let mut ancestor = absolute.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
                suffix.push(ancestor.file_name().context("invalid copy comparison path")?.to_os_string());
                ancestor = ancestor.parent().context("copy comparison has no existing parent")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut result = ancestor.canonicalize()?;
    for component in suffix.into_iter().rev() { result.push(component); }
    Ok(result)
}

#[cfg(unix)]
fn same_file(first: &Path, second: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(first), std::fs::metadata(second)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}
#[cfg(not(unix))]
fn same_file(_: &Path, _: &Path) -> bool { false }

// An absent target has no inode to compare. Conservatively refuse names that
// the host's canonical case/Unicode comparison considers equivalent, including
// missing suffixes below the same existing parent. This creates no probe files.
#[cfg(target_os = "macos")]
fn equivalent_name(first: &Path, second: &Path) -> Result<bool> {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCharacters(allocator: *const c_void, characters: *const u16,
                                        count: isize) -> *const c_void;
        fn CFStringCompare(first: *const c_void, second: *const c_void, options: usize) -> isize;
        fn CFRelease(value: *const c_void);
    }
    struct StringRef(*const c_void);
    impl StringRef {
        fn new(path: &Path) -> Result<Self> {
            let value = path.to_str().context("copy comparison requires an exact UTF-8 path")?;
            let utf16: Vec<u16> = value.encode_utf16().collect();
            let count = isize::try_from(utf16.len()).context("copy comparison path is too long")?;
            // The Create function copies the live UTF-16 slice; default
            // allocator, CFIndex and UniChar match the installed Darwin SDK.
            let result = unsafe { CFStringCreateWithCharacters(std::ptr::null(), utf16.as_ptr(), count) };
            ensure!(!result.is_null(), "could not compare the copy destination");
            Ok(Self(result))
        }
    }
    impl Drop for StringRef {
        fn drop(&mut self) { unsafe { CFRelease(self.0) }; }
    }
    let first = StringRef::new(first)?;
    let second = StringRef::new(second)?;
    // kCFCompareCaseInsensitive | kCFCompareNonliteral; canonical locale,
    // without ignoring diacritics or width. No filesystem ownership is issued.
    Ok(unsafe { CFStringCompare(first.0, second.0, 1 | 16) } == 0)
}

#[cfg(not(target_os = "macos"))]
fn equivalent_name(first: &Path, second: &Path) -> Result<bool> {
    let first = first.to_str().context("copy comparison requires an exact UTF-8 path")?;
    let second = second.to_str().context("copy comparison requires an exact UTF-8 path")?;
    Ok(first.to_lowercase() == second.to_lowercase())
}

pub(super) fn validate(destination: &Path, canonical: &Path,
    input: Option<&sims_runtime::original_session::player_input::Observation>) -> Result<()>
{
    let target = resolved(destination)?;
    ensure!(!equivalent_name(&target, &resolved(canonical)?)? && !same_file(destination, canonical),
        "Choose another copy path: this destination is the configured Save target.");
    if let Some(input) = input {
        let source = Path::new(&input.requested_path);
        ensure!(!equivalent_name(&target, &resolved(source)?)? && !same_file(destination, source),
            "Choose another copy path: this destination is the loaded input.");
        // Also protect the actual retained descriptor identity after a rename.
        #[cfg(unix)]
        if let (Some(descriptor), Ok(metadata)) = (input.descriptor_after.as_ref(), std::fs::metadata(destination)) {
            use std::os::unix::fs::MetadataExt;
            ensure!(descriptor.device != Some(metadata.dev()) || descriptor.inode != Some(metadata.ino()),
                "Choose another copy path: this destination aliases the loaded input.");
        }
    }
    Ok(())
}
