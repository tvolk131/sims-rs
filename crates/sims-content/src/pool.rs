use crate::index::Member;
use crate::{Error, Result};
use sims_formats::far;
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

#[cfg(not(unix))]
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(not(unix))]
use std::sync::Mutex;

/// Maximum open archive file descriptors retained simultaneously in an
/// [`ArchivePool`], keeping descriptor usage well below default OS limits
/// (e.g. macOS's 256 `RLIMIT_NOFILE`) even on download-heavy installs.
const MAX_OPEN_ARCHIVES: usize = 64;

/// Maximum worker threads spawned by parallel startup helpers in this crate.
/// Capped at 8 so nested or sibling startup stages never over-subscribe the
/// host scheduler while still saturating desktop multi-core I/O and parsing.
pub(crate) fn worker_count(work_items: usize) -> usize {
    if work_items <= 1 {
        return 1;
    }
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(1, 8)
        .min(work_items)
}

#[derive(Debug)]
struct OpenArchive {
    #[cfg(unix)]
    file: File,
    #[cfg(not(unix))]
    file: Mutex<File>,
    file_len: u64,
    last_used: AtomicU64,
}

impl OpenArchive {
    fn open(path: &Path, stamp: u64) -> Result<Self> {
        let io = |e| Error::io(path, e);
        let file = File::open(path).map_err(io)?;
        let file_len = file.metadata().map_err(io)?.len();
        Ok(Self {
            #[cfg(unix)]
            file,
            #[cfg(not(unix))]
            file: Mutex::new(file),
            file_len,
            last_used: AtomicU64::new(stamp),
        })
    }

    fn live_len(&self, path: &Path) -> Result<u64> {
        let io = |e| Error::io(path, e);
        #[cfg(unix)]
        {
            Ok(self.file.metadata().map_err(io)?.len())
        }
        #[cfg(not(unix))]
        {
            let guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
            Ok(guard.metadata().map_err(io)?.len())
        }
    }

    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            self.file.read_exact_at(buf, offset)
        }
        #[cfg(not(unix))]
        {
            let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
            guard.seek(SeekFrom::Start(offset))?;
            guard.read_exact(buf)
        }
    }
}

/// Thread-safe bounded cache of open FAR archive file handles for bulk startup reads.
///
/// On Unix, archive members are read via lock-free positional `pread`
/// (`FileExt::read_exact_at`), eliminating thousands of redundant
/// `open`/`lseek`/`close` syscalls per launch while remaining 100% safe Rust
/// (`unsafe_code = "forbid"`). Each open descriptor caches its `file_len` on
/// open so `read_slice` avoids a redundant `fstat` syscall before every
/// `pread`, validating member bounds against the cached file length while
/// relying on `read_exact_at`'s `UnexpectedEof` check to catch post-open
/// truncation. Open handles are bounded to [`MAX_OPEN_ARCHIVES`] with LRU
/// eviction to avoid exhausting OS file descriptors.
#[derive(Debug)]
pub struct ArchivePool {
    archives: RwLock<BTreeMap<PathBuf, Arc<OpenArchive>>>,
    clock: AtomicU64,
    max_open: usize,
}

impl Default for ArchivePool {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchivePool {
    pub fn new() -> Self {
        Self::with_capacity(MAX_OPEN_ARCHIVES)
    }

    fn with_capacity(max_open: usize) -> Self {
        Self {
            archives: RwLock::new(BTreeMap::new()),
            clock: AtomicU64::new(1),
            max_open: max_open.max(1),
        }
    }

    fn touch(&self, archive: &OpenArchive) {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        archive.last_used.store(stamp, Ordering::Relaxed);
    }

    fn get_or_open(&self, path: &Path) -> Result<Arc<OpenArchive>> {
        if let Ok(guard) = self.archives.read()
            && let Some(archive) = guard.get(path)
        {
            self.touch(archive);
            return Ok(Arc::clone(archive));
        }
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let opened = Arc::new(OpenArchive::open(path, stamp)?);
        if let Ok(mut guard) = self.archives.write() {
            if let Some(existing) = guard.get(path) {
                self.touch(existing);
                return Ok(Arc::clone(existing));
            }
            while guard.len() >= self.max_open {
                let victim = guard
                    .iter()
                    .min_by_key(|(_, entry)| {
                        (
                            Arc::strong_count(entry) > 1,
                            entry.last_used.load(Ordering::Relaxed),
                        )
                    })
                    .map(|(k, _)| k.clone());
                match victim {
                    Some(key) => {
                        guard.remove(&key);
                    }
                    None => break,
                }
            }
            guard.insert(path.to_path_buf(), Arc::clone(&opened));
        }
        Ok(opened)
    }

    /// Reads an archive member's full payload, performing the exact bounds and
    /// size checks of [`crate::Resource::read`].
    pub(crate) fn read_member(&self, path: &Path, member: &Member) -> Result<Vec<u8>> {
        self.read_slice(path, &member.name, member.offset, member.size, member.size)
    }

    /// Reads at most `max_len` leading bytes of an archive member, still
    /// validating the full member bounds against the live archive length.
    pub(crate) fn read_member_head(
        &self,
        path: &Path,
        member: &Member,
        max_len: usize,
    ) -> Result<Vec<u8>> {
        let to_read = member.size.min(max_len as u64);
        self.read_slice(path, &member.name, member.offset, member.size, to_read)
    }

    /// Reads `size` bytes at `offset` from the cached file at `path`.
    /// Used by [`crate::ui_graphics::UiGraphics`].
    pub(crate) fn read_range(&self, path: &Path, offset: u64, size: u64) -> Result<Vec<u8>> {
        let archive = self.get_or_open(path)?;
        let io = |e| Error::io(path, e);
        let mut bytes = vec![0u8; size as usize];
        archive.read_exact_at(offset, &mut bytes).map_err(io)?;
        Ok(bytes)
    }

    /// Streams chunk headers of a FAR member IFF directly through the cached
    /// open descriptor, extracting only chunks whose tags are in `tags`.
    ///
    /// Members up to 256 KiB are read in a single `pread` into `scratch`
    /// (avoiding per-chunk syscall overhead and per-file heap allocations);
    /// larger members stream headers via [`Iff::scan_types_from_reader`] so
    /// multi-megabyte `SPR2` payloads are skipped without reading them.
    pub(crate) fn scan_iff_member(
        &self,
        path: &Path,
        location: &str,
        member: &Member,
        tags: &[[u8; 4]],
        scratch: &mut Vec<u8>,
    ) -> Result<sims_formats::iff::ScannedIff> {
        const SINGLE_READ_THRESHOLD: u64 = 256 * 1024;
        let archive = self.get_or_open(path)?;
        let len = archive.file_len;
        let invalid = |message: String| {
            Error::io(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, message),
            )
        };
        if member
            .offset
            .checked_add(member.size)
            .is_none_or(|end| end > len)
        {
            return Err(invalid(format!("FAR member {} outside file", member.name)));
        }
        if member.size > far::MAX_MEMBER {
            return Err(invalid(format!(
                "{} exceeds the 128 MiB resource limit",
                member.name
            )));
        }
        let base = member.offset;
        let member_size = member.size as usize;
        let name = &member.name;
        if member.size <= SINGLE_READ_THRESHOLD {
            scratch.resize(member_size, 0);
            if let Err(err) = archive.read_exact_at(base, scratch) {
                if err.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Err(invalid(format!("{name} ends early")));
                }
                return Err(Error::io(path, err));
            }
            return sims_formats::iff::Iff::scan_types(scratch, tags)
                .map_err(|e| Error::format(location, e));
        }
        let scanned = sims_formats::iff::Iff::scan_types_from_reader(
            member_size,
            |rel_offset, buf| {
                archive
                    .read_exact_at(base + rel_offset as u64, buf)
                    .map_err(|err| {
                        if err.kind() == std::io::ErrorKind::UnexpectedEof {
                            invalid(format!("{name} ends early"))
                        } else {
                            Error::io(path, err)
                        }
                    })
            },
            tags,
        )
        .map_err(|err| match err {
            sims_formats::iff::ScanError::Io(e) => e,
            sims_formats::iff::ScanError::Format(e) => Error::format(location, e),
        })?;
        // Streaming scans skip non-target payloads (such as a trailing SPR2);
        // probe the member's final byte so post-open file truncation inside a
        // skipped tail is still caught identically to a full read.
        if member.size > 0 {
            let mut tail = [0u8; 1];
            if let Err(err) = archive.read_exact_at(base + member.size - 1, &mut tail) {
                if err.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Err(invalid(format!("{name} ends early")));
                }
                return Err(Error::io(path, err));
            }
        }
        Ok(scanned)
    }

    fn read_slice(
        &self,
        path: &Path,
        name: &str,
        offset: u64,
        full_size: u64,
        to_read: u64,
    ) -> Result<Vec<u8>> {
        let archive = self.get_or_open(path)?;
        // Full reads validate the member's entire extent via `read_exact_at`
        // (avoiding a per-`pread` `fstat` syscall); partial peeks (`to_read <
        // full_size`) check the live file length so truncation past `to_read`
        // is never silently accepted.
        let len = if to_read < full_size {
            archive.live_len(path)?
        } else {
            archive.file_len
        };
        let invalid = |message: String| {
            Error::io(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, message),
            )
        };
        if offset.checked_add(full_size).is_none_or(|end| end > len) {
            return Err(invalid(format!("FAR member {name} outside file")));
        }
        if full_size > far::MAX_MEMBER {
            return Err(invalid(format!(
                "{name} exceeds the 128 MiB resource limit"
            )));
        }
        let mut bytes = vec![0u8; to_read as usize];
        if let Err(err) = archive.read_exact_at(offset, &mut bytes) {
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                return Err(invalid(format!("{name} ends early")));
            }
            return Err(Error::io(path, err));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_file(bytes: &[u8]) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sims-content-pool-test-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        let path = dir.join("sample.far");
        std::fs::write(&path, bytes).expect("write temp archive");
        path
    }

    #[test]
    fn concurrent_member_reads_and_head_peeks_match_exact_slices() {
        let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        let path = temp_file(&payload);
        let pool = ArchivePool::new();
        let members: Vec<Member> = (0..16u64)
            .map(|i| Member {
                name: format!("m{i}.bin"),
                offset: i * 64,
                size: 64,
            })
            .collect();

        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        for member in &members {
                            let full = pool.read_member(&path, member).expect("read_member");
                            let start = member.offset as usize;
                            let end = start + member.size as usize;
                            assert_eq!(full, &payload[start..end]);

                            let head = pool
                                .read_member_head(&path, member, 8)
                                .expect("read_member_head");
                            assert_eq!(head, &payload[start..start + 8]);
                        }
                    })
                })
                .collect();
            for handle in handles {
                handle.join().expect("pool worker panicked");
            }
        });

        let _ = std::fs::remove_dir_all(path.parent().expect("temp parent"));
    }

    #[test]
    fn rejects_out_of_bounds_member_even_for_head_peek() {
        let path = temp_file(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let pool = ArchivePool::new();
        let bad = Member {
            name: "truncated.bcf".to_owned(),
            offset: 4,
            size: 16,
        };
        let err = pool
            .read_member_head(&path, &bad, 4)
            .expect_err("out-of-bounds member must fail");
        assert!(
            err.to_string().contains("outside file"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("temp parent"));
    }

    #[test]
    fn head_peek_rejects_member_truncated_after_handle_cached() {
        let path = temp_file(&[0u8; 32]);
        let pool = ArchivePool::new();
        let member = Member {
            name: "anim.bcf".to_owned(),
            offset: 0,
            size: 32,
        };
        assert_eq!(
            pool.read_member_head(&path, &member, 8)
                .expect("initial peek"),
            vec![0u8; 8]
        );
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open for truncate");
        file.set_len(12).expect("truncate archive");
        drop(file);

        let peek_err = pool
            .read_member_head(&path, &member, 8)
            .expect_err("post-open truncation must be rejected even on 8-byte peek");
        assert!(
            peek_err.to_string().contains("outside file"),
            "unexpected peek error: {peek_err}"
        );

        let read_err = pool
            .read_member(&path, &member)
            .expect_err("post-open truncation must be rejected via read_exact_at EOF");
        assert!(
            read_err.to_string().contains("ends early"),
            "unexpected read error: {read_err}"
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("temp parent"));
    }

    #[test]
    fn evicts_idle_handles_when_capacity_exceeded() {
        let pool = ArchivePool::with_capacity(2);
        let paths: Vec<PathBuf> = (0..5u8).map(|i| temp_file(&[i; 16])).collect();
        let member = Member {
            name: "item.bin".to_owned(),
            offset: 0,
            size: 16,
        };
        for (i, path) in paths.iter().enumerate() {
            let bytes = pool.read_member(path, &member).expect("read archive");
            assert_eq!(bytes, vec![i as u8; 16]);
            assert!(
                pool.archives.read().expect("read lock").len() <= 2,
                "open handle count exceeded capacity"
            );
        }
        for path in paths {
            let _ = std::fs::remove_dir_all(path.parent().expect("temp parent"));
        }
    }
}
