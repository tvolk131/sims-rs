// Authored source-only controls. Execution belongs to the integrating parent.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[test]
fn unsupported_platform_fails_closed() {
    let error = super::read(std::path::Path::new("must-not-be-opened.exe")).unwrap_err();
    assert!(error.to_string().contains("require macOS or Linux"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod supported {
    use super::super::{
        descriptor::{ReadPhase, read_with_hook, validate_read},
        *,
    };
    use sha2::{Digest, Sha256};
    use std::{
        ffi::{CString, OsString},
        fs::{self, OpenOptions},
        io::{Seek, SeekFrom, Write},
        os::{
            fd::AsRawFd,
            unix::{
                ffi::{OsStrExt, OsStringExt},
                fs::{DirBuilderExt, MetadataExt, PermissionsExt, symlink},
            },
        },
        sync::atomic::{AtomicU64, Ordering},
    };

    const BASE: u32 = 0x0040_0000;
    const SECTION: u32 = BASE + 0x1000;
    const MARKER: [u8; 4] = [0x51, 0x72, 0x93, 0xb4];
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            loop {
                let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "sims-initial-file-test-{}-{sequence}",
                    std::process::id()
                ));
                match fs::DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create private test directory: {error}"),
                }
            }
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            path
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn put16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    // Authored I386 PE32: one page-aligned section, 512 raw bytes plus 256
    // initial zero bytes. Sixteen absent directories, no imports or asset data.
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x400];
        bytes[..2].copy_from_slice(b"MZ");
        put32(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        put16(&mut bytes, 0x84, 0x014c);
        put16(&mut bytes, 0x86, 1);
        put16(&mut bytes, 0x94, 224);
        put16(&mut bytes, 0x96, 0x0102);
        let optional = 0x98;
        put16(&mut bytes, optional, 0x010b);
        put32(&mut bytes, optional + 4, 0x200);
        put32(&mut bytes, optional + 12, 0x100);
        put32(&mut bytes, optional + 16, 0x1000);
        put32(&mut bytes, optional + 20, 0x1000);
        put32(&mut bytes, optional + 24, 0x1000);
        put32(&mut bytes, optional + 28, BASE);
        put32(&mut bytes, optional + 32, 0x1000);
        put32(&mut bytes, optional + 36, 0x200);
        put16(&mut bytes, optional + 40, 4);
        put16(&mut bytes, optional + 48, 4);
        put32(&mut bytes, optional + 56, 0x2000);
        put32(&mut bytes, optional + 60, 0x200);
        put16(&mut bytes, optional + 68, 3);
        put32(&mut bytes, optional + 72, 0x10_0000);
        put32(&mut bytes, optional + 76, 0x1000);
        put32(&mut bytes, optional + 80, 0x10_0000);
        put32(&mut bytes, optional + 84, 0x1000);
        put32(&mut bytes, optional + 92, 16);
        let section = optional + 224;
        bytes[section..section + 8].copy_from_slice(b".text\0\0\0");
        put32(&mut bytes, section + 8, 0x300);
        put32(&mut bytes, section + 12, 0x1000);
        put32(&mut bytes, section + 16, 0x200);
        put32(&mut bytes, section + 20, 0x200);
        put32(&mut bytes, section + 36, 0x6000_0020);
        bytes[0x200..0x204].copy_from_slice(&MARKER);
        bytes[0x3ff] = 0xa5;
        bytes
    }

    #[test]
    fn retains_exact_read_bytes_sha_and_descriptor_observation() {
        let temp = Temp::new();
        let bytes = fixture();
        let path = temp.write("authored.exe", &bytes);
        let module = read(&path).unwrap();
        let observation = module.observation();
        let metadata = fs::metadata(&path).unwrap();
        let expected_sha: [u8; 32] = Sha256::digest(&bytes).into();
        assert_eq!(module.image().raw(), bytes);
        assert_eq!(module.image().initial_bytes().len(), 0x2000);
        assert_eq!(observation.sha256, expected_sha);
        assert_eq!(observation.file_size, bytes.len() as u64);
        assert_eq!(observation.bytes_read, bytes.len() as u64);
        assert_eq!(observation.supplied_path.as_os_str(), path.as_os_str());
        assert_eq!(observation.before, observation.after);
        let before = &observation.before;
        assert_eq!(before.device, metadata.dev());
        assert_eq!(before.inode, metadata.ino());
        assert_eq!(before.file_size, metadata.len());
        assert_eq!(before.mode, metadata.mode());
        assert_eq!(before.link_count, metadata.nlink());
        assert_eq!(
            (before.modified_seconds, before.modified_nanoseconds),
            (metadata.mtime(), metadata.mtime_nsec())
        );
        assert_eq!(
            (before.changed_seconds, before.changed_nanoseconds),
            (metadata.ctime(), metadata.ctime_nsec())
        );
        assert_eq!(
            (before.user_id, before.group_id),
            (metadata.uid(), metadata.gid())
        );
    }

    #[test]
    fn preserves_dot_path_and_handles_filesystem_non_utf8_policy() {
        let temp = Temp::new();
        temp.write("authored.exe", &fixture());
        let spelling = temp.0.join(".").join("authored.exe");
        let module = read(&spelling).unwrap();
        assert_eq!(module.observation().supplied_path.as_os_str().as_bytes(), spelling.as_os_str().as_bytes());

        let name = OsString::from_vec(b"authored-\xff.exe".to_vec());
        let spelling = temp.0.join(".").join(&name);
        match fs::write(temp.0.join(name), fixture()) {
            Ok(()) => {
                let module = read(&spelling).unwrap();
                assert_eq!(module.observation().supplied_path.as_os_str().as_bytes(), spelling.as_os_str().as_bytes());
            }
            Err(error) if error.raw_os_error() == Some(libc::EILSEQ) => {
                // Some host filesystems require valid UTF-8 names. That is a
                // genuine open failure, never permission to adopt other bytes.
                assert!(read(&spelling).is_err());
                eprintln!("host filesystem rejects non-UTF-8 filenames with EILSEQ; reader refusal exercised");
            }
            Err(error) => panic!("unexpected authored filename creation failure: {error}"),
        }
    }

    #[test]
    #[ignore = "parent-only explicit installed-file path and fixed input digest; initial-file scope only"]
    fn parent_installed_initial_file_keeps_code_and_constructor_state_unqualified() {
        let path = PathBuf::from(std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
            .expect("parent must explicitly admit the installed input path"));
        let module = read(&path).unwrap();
        let observation = module.observation();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(&observation.sha256), "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
        assert_eq!(observation.file_size, 4_422_663);
        assert_eq!(observation.bytes_read, observation.file_size);
        assert_eq!(observation.before, observation.after);
        assert_eq!(module.image().raw().len(), 4_422_663);
        assert_eq!(module.image().header().image_base, BASE);
        assert_eq!(module.image().header().size_of_image, 3_231_744);
        assert_eq!(module.image().sections().len(), 9);
        let prefix = module.initial_span(0x604850, 0x35).unwrap();
        assert_eq!(hex(&prefix.bytes()[..16]), "aa89f08f1d99a15f250268c2e55e6c3b");
        assert_eq!(prefix.origin(), module.origin());
        assert_eq!(module.initial_span(0x6fedec, 4).unwrap().bytes(), &[0; 4]);
        assert!(module.image().file_backed_rva(0x2fedec, 4).is_err());
        assert!(module.initial_span(BASE - 1, 1).is_err());
        assert!(module.initial_span(0x604850, 0).is_err());
        assert!(module.initial_span(u32::MAX, 2).is_err());
        let again = read(&path).unwrap();
        assert_eq!(again.image().raw(), module.image().raw());
        assert_eq!(again.observation().sha256, observation.sha256);
        assert_ne!(again.origin(), module.origin());
        assert_eq!(module.origin().clone(), prefix.origin());
        println!("{}", serde_json::json!({
            "status":"INSTALLED_INITIAL_FILE_BYTES_EXERCISED",
            "file_sha256":hex(&observation.sha256),"file_bytes":observation.bytes_read,
            "image_bytes":module.image().initial_bytes().len(),
            "sections":module.image().sections().len(),"import_slots":module.image().imports().len(),
            "descriptor_consistent":true,"separate_read_origins_distinct":true,
            "factory_matches_captured_source":false,"counter_stage":"INITIAL_FILE_ZERO_FILL_ONLY",
            "constructor_entry_state_qualified":false,"execution_performed":false,"all_alpha_gates":false,
        }));
    }

    #[test]
    #[ignore = "parent-only fixed installed input; module-image prefix, no driver/CRT/counter execution"]
    fn parent_installed_entry_image_keeps_driver_crt_and_counter_unqualified() {
        let path = PathBuf::from(std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE")
            .expect("parent must explicitly admit the installed input path"));
        let module = read(&path).unwrap();
        let observed = module.observation().clone();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(&observed.sha256), "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
        let origin = module.origin();
        let mut entry = module.begin_entry().unwrap();
        assert!(entry.writes().is_empty() && entry.pending_call().is_none());
        assert_eq!(entry.advance().unwrap(), crate::native_bootstrap::executable::entry::Cut::BeforeProtectionDriver);
        assert_eq!(entry.module().observation(), &observed);
        assert_eq!(entry.module().initial_span(0x71406e, 5).unwrap().bytes(), &[0x55, 0x8b, 0xec, 0x60, 0xbb]);
        assert_eq!(entry.current_span(0x71406e, 5).unwrap().bytes(), &[0xe9, 0xf1, 0x42, 0xf2, 0xff]);
        assert_eq!(entry.current_span(0x6fedec, 4).unwrap().bytes(), &[0; 4]);
        assert_eq!(entry.current_span(0x7142c8, 4).unwrap().bytes(), &0x0031_4320u32.to_le_bytes());
        let call = entry.pending_call().unwrap().clone();
        assert_eq!((call.instruction(), call.target(), call.arguments()), (0x7140b6, 0x713ce0, [0, 0x713fd9]));
        assert_eq!(call.origin(), origin);
        assert!(entry.accepts_call(&call));
        assert_eq!(entry.writes().len(), 2);
        assert_eq!(entry.writes()[0].instruction(), 0x71409e);
        assert_eq!(entry.writes()[1].instruction(), 0x7140a1);
        assert_eq!(entry.advance().unwrap(), crate::native_bootstrap::executable::entry::Cut::BeforeProtectionDriver);
        assert!(entry.pending_call().unwrap().same_call(&call));
        assert_eq!(entry.writes().len(), 2);
        println!("{}", serde_json::json!({
            "status":"INSTALLED_ENTRY_IMAGE_TWO_STORES_EXERCISED_BEFORE_DRIVER",
            "file_sha256":hex(&observed.sha256),"entry_stores":2,
            "counter_stage":"INITIAL_MAPPING_UNCHANGED_NOT_CONSTRUCTOR_ENTRY",
            "iat_stage":"INITIAL_LOOKUP_RVA_NOT_CALLABLE","cpu_or_stack_executed":false,
            "driver_executed":false,"crt_initialized":false,
            "constructor_entry_state_qualified":false,"all_alpha_gates":false,
        }));
    }

    #[test]
    fn descriptor_has_nonblocking_and_close_on_exec_flags() {
        let temp = Temp::new();
        let path = temp.write("flags.exe", &fixture());
        let mut checked = false;
        read_with_hook(&path, |phase, file| {
            if let ReadPhase::BeforeRead = phase {
                // SAFETY: both fcntl commands inspect the live borrowed fd and
                // take no variadic argument or ownership of the descriptor.
                let status = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
                let descriptor = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
                assert!(status >= 0 && descriptor >= 0);
                assert_ne!(status & libc::O_NONBLOCK, 0);
                assert_ne!(descriptor & libc::FD_CLOEXEC, 0);
                checked = true;
            }
            Ok(())
        })
        .unwrap();
        assert!(checked);
    }

    #[test]
    fn independent_same_byte_loads_have_distinct_origins() {
        let temp = Temp::new();
        let path = temp.write("same.exe", &fixture());
        let first = read(&path).unwrap();
        let second = read(&path).unwrap();
        assert_eq!(first.observation(), second.observation());
        assert_eq!(first.image().raw(), second.image().raw());
        assert_ne!(first.origin(), second.origin());
        assert_eq!(first.origin(), first.origin().clone());
        let first_view = first.initial_span(SECTION, MARKER.len()).unwrap();
        let second_view = second.initial_span(SECTION, MARKER.len()).unwrap();
        assert_eq!(first_view.address(), second_view.address());
        assert_eq!(first_view.bytes(), second_view.bytes());
        assert_ne!(first_view.origin(), second_view.origin());
    }

    #[test]
    fn retained_token_cannot_become_a_later_loads_identity() {
        let temp = Temp::new();
        let path = temp.write("lifetime.exe", &fixture());
        let origin = {
            let first = read(&path).unwrap();
            let origin = first.origin();
            assert_eq!(first.initial_span(SECTION, 1).unwrap().origin(), origin);
            origin
        };
        let clone = origin.clone();
        let later = read(&path).unwrap();
        assert_eq!(origin, clone);
        assert_ne!(origin, later.origin());
        assert_ne!(clone, later.initial_span(SECTION, 1).unwrap().origin());
        assert_eq!(format!("{origin:?}"), "ModuleOrigin(..)");
    }

    #[test]
    fn initial_views_borrow_the_images_bytes_and_origin() {
        let temp = Temp::new();
        let path = temp.write("views.exe", &fixture());
        let module = read(&path).unwrap();
        let view = module.initial_span(SECTION, MARKER.len()).unwrap();
        assert_eq!(view.address(), SECTION);
        assert_eq!(view.bytes(), MARKER);
        assert!(std::ptr::eq(
            view.bytes().as_ptr(),
            module.image().initial_va(SECTION, MARKER.len()).unwrap().as_ptr()
        ));
        assert_eq!(view.origin(), module.origin());
        assert_eq!(module.initial_span(BASE, 2).unwrap().bytes(), b"MZ");
        assert_eq!(
            module.initial_span(SECTION + 0x1ff, 1).unwrap().bytes(),
            &[0xa5]
        );
        // Initial-file zero-fill only; this makes no constructor-entry claim.
        let zero_fill = module.initial_span(SECTION + 0x200, 0x100).unwrap();
        assert!(zero_fill.bytes().iter().all(|byte| *byte == 0));
        assert_eq!(zero_fill.origin(), view.origin());
    }

    #[test]
    fn initial_views_reject_empty_overflow_gap_tail_and_crossing_queries() {
        let temp = Temp::new();
        let path = temp.write("ranges.exe", &fixture());
        let module = read(&path).unwrap();
        for (address, length) in [
            (SECTION, 0),
            (BASE - 1, 1),
            (BASE + 0x200, 1),
            (BASE + 0x1ff, 2),
            (SECTION + 0x300, 1),
            (SECTION + 0x2ff, 2),
            (BASE + 0x2000, 1),
            (u32::MAX, 2),
            (SECTION, usize::MAX),
        ] {
            assert!(
                module.initial_span(address, length).is_err(),
                "{address:#x}, {length}"
            );
        }
    }

    #[test]
    fn path_replacement_after_read_does_not_rebind_existing_views() {
        let temp = Temp::new();
        let bytes = fixture();
        let path = temp.write("replace.exe", &bytes);
        let module = read(&path).unwrap();
        let view = module.initial_span(SECTION, 4).unwrap();
        let original_observation = module.observation().clone();
        let mut replacement = fixture();
        replacement[0x200..0x204].copy_from_slice(b"next");
        let replacement_path = temp.write("replacement.exe", &replacement);
        fs::rename(replacement_path, &path).unwrap();
        let later = read(&path).unwrap();
        assert_eq!(module.image().raw(), bytes);
        assert_eq!(module.observation(), &original_observation);
        assert_eq!(view.bytes(), MARKER);
        assert_eq!(view.origin(), module.origin());
        assert_eq!(later.initial_span(SECTION, 4).unwrap().bytes(), b"next");
        assert_ne!(module.origin(), later.origin());
        assert_ne!(
            module.observation().before.inode,
            later.observation().before.inode
        );
    }

    #[test]
    fn directory_replacement_during_read_keeps_the_original_open_descriptor() {
        let temp = Temp::new();
        let selected = temp.0.join("selected");
        let alternative = temp.0.join("alternative");
        let retained = temp.0.join("retained");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&alternative).unwrap();
        let original = fixture();
        let mut replacement = fixture();
        replacement[0x200..0x204].copy_from_slice(b"next");
        let path = selected.join("authored.exe");
        fs::write(&path, &original).unwrap();
        fs::write(alternative.join("authored.exe"), &replacement).unwrap();
        let original_inode = fs::metadata(&path).unwrap().ino();
        let module = read_with_hook(&path, |phase, _| {
            if let ReadPhase::BeforeRead = phase {
                // Moving a parent directory leaves the child's descriptor and
                // inode metadata unchanged. A path reopen would read "next".
                fs::rename(&selected, &retained)?;
                fs::rename(&alternative, &selected)?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(module.image().raw(), original);
        assert_eq!(module.observation().before.inode, original_inode);
        assert_eq!(module.observation().before, module.observation().after);
        assert_eq!(module.initial_span(SECTION, 4).unwrap().bytes(), MARKER);
        let later = read(&path).unwrap();
        assert_eq!(later.image().raw(), replacement);
        assert_ne!(module.origin(), later.origin());
    }

    #[test]
    fn in_place_write_and_unlink_after_read_leave_retained_views_unchanged() {
        let temp = Temp::new();
        let bytes = fixture();
        let path = temp.write("changes.exe", &bytes);
        let module = read(&path).unwrap();
        let view = module.initial_span(SECTION, 4).unwrap();
        let observation = module.observation().clone();
        {
            let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
            writer.seek(SeekFrom::Start(0x200)).unwrap();
            writer.write_all(b"edit").unwrap();
            writer.set_len(1).unwrap();
        }
        fs::remove_file(&path).unwrap();
        assert!(read(&path).is_err());
        assert_eq!(view.bytes(), MARKER);
        assert_eq!(view.origin(), module.origin());
        assert_eq!(module.image().raw(), bytes);
        assert_eq!(module.observation(), &observation);
    }

    #[test]
    fn malformed_empty_and_truncated_files_return_no_module() {
        let temp = Temp::new();
        for (index, bytes) in [
            vec![],
            b"MZ".to_vec(),
            vec![0; 512],
            fixture()[..0x201].to_vec(),
        ]
        .into_iter()
        .enumerate()
        {
            let path = temp.write(&format!("malformed-{index}.exe"), &bytes);
            let error = read(&path).unwrap_err();
            assert!(error.to_string().contains("parse bounded PE32"));
        }
    }

    #[test]
    fn missing_paths_and_directories_are_refused() {
        let temp = Temp::new();
        assert!(read(&temp.0.join("absent.exe")).is_err());
        let directory = temp.0.join("directory.exe");
        fs::create_dir(&directory).unwrap();
        assert!(
            read(&directory)
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );
    }

    #[test]
    fn final_symlinks_to_regular_directory_and_absent_targets_are_refused() {
        let temp = Temp::new();
        let regular = temp.write("regular.exe", &fixture());
        for (index, target) in [regular, temp.0.clone(), temp.0.join("absent")]
            .into_iter()
            .enumerate()
        {
            let link = temp.0.join(format!("link-{index}.exe"));
            symlink(target, &link).unwrap();
            assert!(read(&link).is_err());
        }
    }

    #[test]
    fn fifo_without_writer_and_symlink_to_fifo_are_refused() {
        let temp = Temp::new();
        let fifo = temp.0.join("unwritten.fifo");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is NUL-terminated and remains alive during mkfifo. Only
        // this test's new private temporary directory is affected.
        let result = unsafe { libc::mkfifo(name.as_ptr(), 0o600) };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
        let error = read(&fifo).unwrap_err();
        assert!(error.to_string().contains("not a regular file"));
        let link = temp.0.join("fifo-link");
        symlink(&fifo, &link).unwrap();
        assert!(read(&link).is_err());
    }

    #[test]
    fn oversize_descriptor_is_rejected_before_any_read_hook() {
        let temp = Temp::new();
        let path = temp.write("large.exe", &fixture());
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(pe32::MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        let mut reached_read = false;
        let error = read_with_hook(&path, |_, _| {
            reached_read = true;
            Ok(())
        })
        .unwrap_err();
        assert!(!reached_read);
        assert!(error.to_string().contains("descriptor length exceeds byte bound"));
    }

    #[test]
    fn growth_is_bounded_to_cap_plus_one_and_rejected() {
        let temp = Temp::new();
        let path = temp.write("growing.exe", &fixture());
        let mut observed_count = None;
        let error = read_with_hook(&path, |phase, _| {
            match phase {
                ReadPhase::BeforeRead => {
                    OpenOptions::new().write(true).open(&path)?
                        .set_len(pe32::MAX_FILE_BYTES as u64 + 4096)?;
                }
                ReadPhase::AfterRead { bytes_read } => observed_count = Some(bytes_read),
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(observed_count, Some(pe32::MAX_FILE_BYTES + 1));
        assert!(error.to_string().contains("read exceeds byte bound"));
    }

    #[test]
    fn truncation_between_descriptor_stat_and_read_is_rejected() {
        let temp = Temp::new();
        let bytes = fixture();
        let path = temp.write("shrinking.exe", &bytes);
        let mut observed_count = None;
        let error = read_with_hook(&path, |phase, _| {
            match phase {
                ReadPhase::BeforeRead => {
                    OpenOptions::new().write(true).open(&path)?.set_len(2)?;
                }
                ReadPhase::AfterRead { bytes_read } => observed_count = Some(bytes_read),
            }
            Ok(())
        })
        .unwrap_err();
        assert_eq!(observed_count, Some(2));
        assert!(error.to_string().contains("descriptor metadata changed"));
    }

    #[test]
    fn deterministic_mode_change_after_read_rejects_otherwise_valid_bytes() {
        let temp = Temp::new();
        let path = temp.write("mode-change.exe", &fixture());
        let error = read_with_hook(&path, |phase, file| {
            if let ReadPhase::AfterRead { bytes_read } = phase {
                assert_eq!(bytes_read, 0x400);
                let old_mode = file.metadata()?.permissions().mode();
                file.set_permissions(fs::Permissions::from_mode(old_mode ^ 0o100))?;
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("descriptor metadata changed"));
    }

    #[test]
    fn consistency_compares_every_descriptor_field_and_exact_read_length() {
        let temp = Temp::new();
        let path = temp.write("metadata.exe", &fixture());
        let module = read(&path).unwrap();
        let before = &module.observation().before;
        validate_read(before, before, 0x400).unwrap();
        assert!(validate_read(before, before, 0x3ff).is_err());
        assert!(validate_read(before, before, 0x401).is_err());
        let changes: [(&str, fn(&mut DescriptorMetadata)); 11] = [
            ("device", |m| m.device ^= 1),
            ("inode", |m| m.inode ^= 1),
            ("file_size", |m| m.file_size ^= 1),
            ("mode", |m| m.mode ^= 1),
            ("link_count", |m| m.link_count ^= 1),
            ("modified_seconds", |m| m.modified_seconds ^= 1),
            ("modified_nanoseconds", |m| m.modified_nanoseconds ^= 1),
            ("changed_seconds", |m| m.changed_seconds ^= 1),
            ("changed_nanoseconds", |m| m.changed_nanoseconds ^= 1),
            ("user_id", |m| m.user_id ^= 1),
            ("group_id", |m| m.group_id ^= 1),
        ];
        for (name, change) in changes {
            let mut after = before.clone();
            change(&mut after);
            let error = validate_read(before, &after, 0x400).expect_err(name);
            assert!(error.to_string().contains("descriptor metadata changed"));
        }
    }
}
