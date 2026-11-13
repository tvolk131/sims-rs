// Authored source-only PE-file controls. ALL UNRUN by this implementation worker.
// Filter: native_bootstrap::executable::entry::tests::
// The parent alone integrates executable.rs, compiles and executes these tests.

use super::*;
use crate::native_bootstrap::executable::read;
use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const OPTIONAL: usize = 0x98;
const TEXT_RECORD: usize = OPTIONAL + 224;
const STXT_RECORD: usize = TEXT_RECORD + 40;
const STXT_RAW: usize = 0x800;
const STXT_RVA: u32 = 0x0031_3000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        loop {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sims-entry-image-test-{}-{sequence}", std::process::id()
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create private entry test directory: {error}"),
            }
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
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

fn file_offset(va: u32) -> usize {
    STXT_RAW + usize::try_from(va - BASE - STXT_RVA).unwrap()
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes().chunks_exact(2).map(|pair| {
        u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
    }).collect()
}

// Authored I386 PE32 EXE with two sections and all sixteen directories absent.
// The low section only supplies mapped coverage for the patched destination;
// its filler is not protected code. stxt371 contains the contract's entry
// instructions, zero flag and forwarding jump, plus arbitrary unrelated filler.
// There are no private assets, imports, callable driver bodies or CRT state.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x1a00];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 0x84, 0x014c);
    put16(&mut bytes, 0x86, 2);
    put16(&mut bytes, 0x94, 224);
    put16(&mut bytes, 0x96, 0x0102);
    put16(&mut bytes, OPTIONAL, 0x010b);
    put32(&mut bytes, OPTIONAL + 4, 0x1800);
    put32(&mut bytes, OPTIONAL + 12, 0x800);
    put32(&mut bytes, OPTIONAL + 16, 0x0031_406e);
    put32(&mut bytes, OPTIONAL + 20, 0x0023_8000);
    put32(&mut bytes, OPTIONAL + 24, STXT_RVA);
    put32(&mut bytes, OPTIONAL + 28, 0x0040_0000);
    put32(&mut bytes, OPTIONAL + 32, 0x1000);
    put32(&mut bytes, OPTIONAL + 36, 0x200);
    put16(&mut bytes, OPTIONAL + 40, 4);
    put16(&mut bytes, OPTIONAL + 48, 4);
    put32(&mut bytes, OPTIONAL + 56, 0x0031_5000);
    put32(&mut bytes, OPTIONAL + 60, 0x200);
    put16(&mut bytes, OPTIONAL + 68, 3);
    put32(&mut bytes, OPTIONAL + 72, 0x10_0000);
    put32(&mut bytes, OPTIONAL + 76, 0x1000);
    put32(&mut bytes, OPTIONAL + 80, 0x10_0000);
    put32(&mut bytes, OPTIONAL + 84, 0x1000);
    put32(&mut bytes, OPTIONAL + 92, 16);
    bytes[TEXT_RECORD..TEXT_RECORD + 8].copy_from_slice(b".text\0\0\0");
    put32(&mut bytes, TEXT_RECORD + 8, 0x800);
    put32(&mut bytes, TEXT_RECORD + 12, 0x0023_8000);
    put32(&mut bytes, TEXT_RECORD + 16, 0x600);
    put32(&mut bytes, TEXT_RECORD + 20, 0x200);
    put32(&mut bytes, TEXT_RECORD + 36, 0x6000_0020);
    bytes[STXT_RECORD..STXT_RECORD + 8].copy_from_slice(b"stxt371\0");
    put32(&mut bytes, STXT_RECORD + 8, 0x1800);
    put32(&mut bytes, STXT_RECORD + 12, STXT_RVA);
    put32(&mut bytes, STXT_RECORD + 16, 0x1200);
    put32(&mut bytes, STXT_RECORD + 20, STXT_RAW as u32);
    put32(&mut bytes, STXT_RECORD + 36, 0xe000_0020);
    for (index, byte) in bytes[0x200..].iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(29).wrapping_add(0x53);
    }
    let source = hex_bytes(concat!(
        "558bec60bb6e407100b80d40710033c98a0885c9740cb8e44071002bc383e805eb0e",
        "51b92b4171008bc12bc303410159c603e98943015168d93f710033c085c974058b4508",
        "eb0050e825fcffff"
    ));
    assert_eq!(source.len(), 77);
    let at = file_offset(0x0071_406e);
    bytes[at..at + source.len()].copy_from_slice(&source);
    let at = file_offset(0x0071_412b);
    bytes[at..at + 5].copy_from_slice(&[0xe9, 0x34, 0x42, 0xf2, 0xff]);
    bytes[file_offset(0x0071_400d)] = 0;
    bytes
}

// Every rejection control goes through a real new file and executable::read.
// Buffer addresses, full bytes and observations prove the consumed Module is
// returned intact; no replacement read, origin, synthetic image or adoption.
fn assert_refused(bytes: &[u8], message: &str) {
    let temp = Temp::new();
    let path = temp.write("refused.exe", bytes);
    let module = read(&path).expect("control must reach Entry via the real PE parser");
    let origin = module.origin();
    let observation = module.observation().clone();
    let raw_pointer = module.image().raw().as_ptr();
    let initial_pointer = module.image().initial_bytes().as_ptr();
    let initial = module.image().initial_bytes().to_vec();
    let error = Entry::begin(module).expect_err(message);
    assert!(error.error.to_string().contains(message), "{}", error.error);
    assert_eq!(error.module.origin(), origin);
    assert_eq!(error.module.observation(), &observation);
    assert_eq!(error.module.image().raw(), bytes);
    assert_eq!(error.module.image().initial_bytes(), initial);
    assert_eq!(error.module.image().raw().as_ptr(), raw_pointer);
    assert_eq!(error.module.image().initial_bytes().as_ptr(), initial_pointer);
    assert_eq!(error.module.observation().before.inode, fs::metadata(&path).unwrap().ino());
    // Even after unlink, the returned owner still has the exact consumed bytes.
    fs::remove_file(path).unwrap();
    assert_eq!(error.module.initial_span(error.module.image().header().image_base, 2).unwrap().bytes(), b"MZ");
    assert_eq!(error.module.image().initial_bytes(), initial);
}

#[test]
fn consumes_actual_module_and_changes_only_five_bytes_with_exact_preimages() {
    let temp = Temp::new();
    let bytes = fixture();
    let path = temp.write("initial.exe", &bytes);
    let module = read(&path).unwrap();
    let observation = module.observation().clone();
    let origin = module.origin();
    let raw_pointer = module.image().raw().as_ptr();
    let initial_pointer = module.image().initial_bytes().as_ptr();
    let initial = module.image().initial_bytes().to_vec();
    let mut entry = Entry::begin(module).unwrap();
    assert_eq!(entry.module().origin(), origin);
    assert_eq!(entry.module().observation(), &observation);
    assert_eq!(entry.module().image().raw().as_ptr(), raw_pointer);
    assert_eq!(entry.module().image().initial_bytes().as_ptr(), initial_pointer);
    assert_ne!(entry.image.as_ptr(), initial_pointer);
    assert_eq!(entry.image, initial);
    assert!(entry.writes().is_empty());
    assert!(entry.pending_call().is_none());
    assert_eq!(entry.current_span(ENTRY, 5).unwrap().bytes(), &[0x55, 0x8b, 0xec, 0x60, 0xbb]);
    assert_eq!(entry.advance().unwrap(), Cut::BeforeProtectionDriver);
    let patched = [0xe9, 0xf1, 0x42, 0xf2, 0xff];
    let mut expected = initial.clone();
    expected[0x0031_406e..0x0031_4073].copy_from_slice(&patched);
    assert_eq!(entry.image, expected); // Entire buffer, including all gaps/BSS.
    let differences: Vec<_> = initial.iter().zip(&entry.image).enumerate()
        .filter_map(|(index, (before, after))| (before != after).then_some(index)).collect();
    assert_eq!(differences, (0x0031_406e..0x0031_4073).collect::<Vec<_>>());
    assert_eq!(entry.module().image().raw(), bytes);
    assert_eq!(entry.module().image().initial_bytes(), initial);
    assert_eq!(entry.module().observation(), &observation);
    assert_eq!(entry.module().initial_span(ENTRY, 5).unwrap().bytes(), &[0x55, 0x8b, 0xec, 0x60, 0xbb]);
    let current = entry.current_span(ENTRY, 5).unwrap();
    assert_eq!(current.address(), ENTRY);
    assert_eq!(current.bytes(), patched);
    assert_eq!(current.origin(), origin);
    assert!(std::ptr::eq(current.bytes().as_ptr(), entry.image[0x0031_406e..].as_ptr()));
    let history = entry.writes();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].instruction(), 0x0071_409e);
    assert_eq!(history[0].address(), 0x0071_406e);
    assert_eq!(history[0].before(), &[0x55]);
    assert_eq!(history[0].after(), &[0xe9]);
    assert_eq!(history[1].instruction(), 0x0071_40a1);
    assert_eq!(history[1].address(), 0x0071_406f);
    assert_eq!(history[1].before(), &[0x8b, 0xec, 0x60, 0xbb]);
    assert_eq!(history[1].after(), &[0xf1, 0x42, 0xf2, 0xff]);
    let call = entry.pending_call().unwrap();
    assert_eq!(call.instruction(), 0x0071_40b6);
    assert_eq!(call.target(), 0x0071_3ce0);
    assert_eq!(call.arguments(), [0, 0x0071_3fd9]);
    assert_eq!(call.origin(), origin);
    assert!(entry.accepts_call(call));
    assert!(entry.current_span(call.arguments()[1], 1).is_ok());
}

#[test]
fn derives_patch_from_actual_forwarding_dword_and_wraps_the_jump_target() {
    let temp = Temp::new();
    let path = temp.write("wrapping.exe", &fixture());
    let mut entry = Entry::begin(read(&path).unwrap()).unwrap();
    let jump = entry.current_span(0x0071_412b, 5).unwrap();
    let source_operand = u32::from_le_bytes(jump.bytes()[1..5].try_into().unwrap());
    assert_eq!(source_operand, 0xfff2_4234);
    let expected = 0x0071_412bu32.wrapping_sub(0x0071_406e).wrapping_add(source_operand);
    assert_eq!(expected, 0xfff2_42f1);
    entry.advance().unwrap();
    let patch = entry.current_span(0x0071_406f, 4).unwrap();
    let actual = u32::from_le_bytes(patch.bytes().try_into().unwrap());
    assert_eq!(actual, expected);
    assert!(0x0071_4073u32.checked_add(actual).is_none());
    assert_eq!(0x0071_4073u32.wrapping_add(actual), 0x0063_8364);
    // No cursor follows that address; it remains before the unresolved driver.
    assert_eq!(entry.pending_call().unwrap().instruction(), 0x0071_40b6);
}

#[test]
fn duplicate_advance_retains_exact_cut_call_image_and_history_without_replay() {
    let temp = Temp::new();
    let path = temp.write("duplicate.exe", &fixture());
    let mut entry = Entry::begin(read(&path).unwrap()).unwrap();
    let cut = entry.advance().unwrap();
    let call = entry.pending_call().unwrap().clone();
    let call_address = entry.pending_call().unwrap() as *const ProtectionCall;
    let history_address = entry.writes().as_ptr();
    let image_address = entry.image.as_ptr();
    let image = entry.image.clone();
    for _ in 0..3 {
        assert_eq!(entry.advance().unwrap(), cut);
        assert!(call.same_call(entry.pending_call().unwrap()));
        assert!(entry.accepts_call(&call));
        assert_eq!(entry.pending_call().unwrap() as *const ProtectionCall, call_address);
        assert_eq!(entry.writes().as_ptr(), history_address);
        assert_eq!(entry.writes().len(), 2);
        assert_eq!(entry.writes()[0].before(), &[0x55]);
        assert_eq!(entry.writes()[1].before(), &[0x8b, 0xec, 0x60, 0xbb]);
        assert_eq!(entry.image.as_ptr(), image_address);
        assert_eq!(entry.image, image);
    }
    // Moving the owner retains allocation identity and actual Module lifetime.
    let moved = Box::new(entry);
    assert!(moved.accepts_call(&call));
    assert!(moved.pending_call().unwrap().same_call(&call.clone()));
}

#[test]
fn nonzero_flags_are_refused_before_any_image_effect_with_the_actual_owner() {
    for flag in [1, 0x80, 0xff] {
        let mut bytes = fixture();
        bytes[file_offset(0x0071_400d)] = flag;
        assert_refused(&bytes, "nonzero entry flag");
    }
}

#[test]
fn every_entry_instruction_and_forwarding_byte_is_validated_before_effects() {
    for index in 0..77 {
        let mut bytes = fixture();
        bytes[file_offset(0x0071_406e) + index] ^= 1;
        assert_refused(&bytes, "unsupported initial entry instructions");
    }
    for index in 0..5 {
        let mut bytes = fixture();
        bytes[file_offset(0x0071_412b) + index] ^= 1;
        assert_refused(&bytes, "unsupported initial forwarding jump");
    }
}

#[test]
fn wrong_base_entry_and_dll_kind_return_the_consumed_module() {
    let mut bytes = fixture();
    put32(&mut bytes, OPTIONAL + 28, 0x0041_0000);
    assert_refused(&bytes, "unsupported entry preferred base");
    let mut bytes = fixture();
    put32(&mut bytes, OPTIONAL + 16, 0x0031_406f);
    assert_refused(&bytes, "unsupported initial entry RVA");
    let mut bytes = fixture();
    put16(&mut bytes, 0x96, 0x2102);
    assert_refused(&bytes, "requires EXE, not DLL");
}

#[test]
fn relocation_and_tls_directories_are_refused_without_running_their_contents() {
    for index in [5, 9] {
        let mut bytes = fixture();
        put32(&mut bytes, OPTIONAL + 96 + index * 8, 0x0031_3100);
        put32(&mut bytes, OPTIONAL + 100 + index * 8, 4);
        assert_refused(&bytes, "absent relocation/TLS directories");
    }
}

#[test]
fn stxt_identity_and_exact_read_write_execute_profile_are_required() {
    let mut bytes = fixture();
    bytes[STXT_RECORD..STXT_RECORD + 8].copy_from_slice(b"stxt372\0");
    assert_refused(&bytes, "requires stxt371");
    let mut bytes = fixture();
    bytes[TEXT_RECORD..TEXT_RECORD + 8].copy_from_slice(b"stxt371\0");
    assert_refused(&bytes, "unsupported stxt371 identity or characteristics");
    for characteristics in [0x6000_0020, 0xc000_0020, 0xa000_0020, 0xe000_0040] {
        let mut bytes = fixture();
        put32(&mut bytes, STXT_RECORD + 36, characteristics);
        assert_refused(&bytes, "unsupported stxt371 identity or characteristics");
    }
    let mut bytes = fixture();
    put32(&mut bytes, TEXT_RECORD + 36, 0x4000_0040);
    assert_refused(&bytes, "lacks required section access");
}

#[test]
fn source_jump_driver_and_patched_target_require_real_mapped_extents() {
    for virtual_size in [0x1080, 0x112e] {
        let mut bytes = fixture();
        put32(&mut bytes, STXT_RECORD + 8, virtual_size);
        put32(&mut bytes, STXT_RECORD + 16, 0x1000);
        // The real parser accepts the initial entry byte. Entry refuses the
        // whole source or forwarding span before any effect or byte adoption.
        assert_refused(&bytes, "RVA range is not one declared span");
    }
    let mut bytes = fixture();
    // Move stxt to 714000, retaining source/flag/jump in its new raw locations.
    // Driver713ce0 and config713fd9 then fall in a genuine mapping gap.
    bytes.copy_within(file_offset(0x0071_406e)..file_offset(0x0071_406e) + 77, STXT_RAW + 0x6e);
    bytes.copy_within(file_offset(0x0071_412b)..file_offset(0x0071_412b) + 5, STXT_RAW + 0x12b);
    bytes[STXT_RAW + 0x0d] = 0;
    put32(&mut bytes, STXT_RECORD + 12, 0x0031_4000);
    put32(&mut bytes, STXT_RECORD + 8, 0x800);
    put32(&mut bytes, STXT_RECORD + 16, 0x600);
    assert_refused(&bytes, "RVA range is not one declared span");
    let mut bytes = fixture();
    put32(&mut bytes, TEXT_RECORD + 12, 0x0023_7000);
    assert_refused(&bytes, "RVA range is not one declared span");
}

#[test]
fn stxt_name_elsewhere_cannot_qualify_the_real_entry_section() {
    let mut bytes = fixture();
    bytes[TEXT_RECORD..TEXT_RECORD + 8].copy_from_slice(b"stxt371\0");
    put32(&mut bytes, TEXT_RECORD + 36, 0xe000_0020);
    bytes[STXT_RECORD..STXT_RECORD + 8].copy_from_slice(b".other\0\0");
    assert_refused(&bytes, "must be inside stxt371");
}

#[test]
fn current_spans_borrow_live_bytes_and_check_empty_overflow_gaps_tails_and_crossings() {
    let temp = Temp::new();
    let path = temp.write("spans.exe", &fixture());
    let mut entry = Entry::begin(read(&path).unwrap()).unwrap();
    for after_advance in [false, true] {
        if after_advance { entry.advance().unwrap(); }
        assert_eq!(entry.current_span(BASE, 2).unwrap().bytes(), b"MZ");
        let zero_fill = entry.current_span(0x0071_4200, 0x600).unwrap();
        assert!(zero_fill.bytes().iter().all(|byte| *byte == 0));
        assert_eq!(zero_fill.origin(), entry.module().origin());
        assert_eq!(entry.current_span(0x0071_47ff, 1).unwrap().bytes(), &[0]);
        for (address, length) in [
            (ENTRY, 0), (BASE - 1, 1), (BASE + 0x200, 1), (BASE + 0x1ff, 2),
            (0x0063_8800, 1), (0x0071_4800, 1), (0x0071_47ff, 2),
            (BASE + 0x0031_5000, 1), (u32::MAX, 2), (ENTRY, usize::MAX),
        ] {
            assert!(entry.current_span(address, length).is_err(), "{address:#x}, {length}");
        }
    }
}

#[test]
fn independent_same_file_loads_and_foreign_calls_keep_distinct_identity() {
    let temp = Temp::new();
    let path = temp.write("identical.exe", &fixture());
    let first = read(&path).unwrap();
    let second = read(&path).unwrap();
    assert_eq!(first.observation(), second.observation());
    assert_eq!(first.image().raw(), second.image().raw());
    assert_ne!(first.origin(), second.origin());
    let mut first = Entry::begin(first).unwrap();
    let mut second = Entry::begin(second).unwrap();
    first.advance().unwrap();
    let call = first.pending_call().unwrap().clone();
    assert!(first.accepts_call(&call));
    assert!(!second.accepts_call(&call));
    assert!(second.pending_call().is_none());
    second.advance().unwrap();
    let foreign = second.pending_call().unwrap();
    assert_eq!(call.instruction(), foreign.instruction());
    assert_eq!(call.target(), foreign.target());
    assert_eq!(call.arguments(), foreign.arguments());
    assert_ne!(call.origin(), foreign.origin());
    assert!(!call.same_call(foreign));
    assert!(!first.accepts_call(foreign));
    assert!(!second.accepts_call(&call));
    assert!(foreign.same_call(&foreign.clone()));
}

#[test]
fn a_call_retained_after_its_owner_drops_cannot_identify_a_later_entry() {
    let temp = Temp::new();
    let path = temp.write("historical.exe", &fixture());
    let stale = {
        let mut owner = Entry::begin(read(&path).unwrap()).unwrap();
        owner.advance().unwrap();
        owner.pending_call().unwrap().clone()
    };
    let observation_clone = stale.clone();
    assert!(stale.same_call(&observation_clone));
    let mut later = Entry::begin(read(&path).unwrap()).unwrap();
    assert_ne!(stale.origin(), later.module().origin());
    assert!(!later.accepts_call(&stale));
    later.advance().unwrap();
    assert!(!later.accepts_call(&stale));
    assert!(!later.pending_call().unwrap().same_call(&stale));
    assert!(stale.same_call(&observation_clone));
}

#[test]
fn entry_retains_actual_module_observation_after_path_replacement_and_unlink() {
    let temp = Temp::new();
    let bytes = fixture();
    let path = temp.write("retained.exe", &bytes);
    let module = read(&path).unwrap();
    let observation = module.observation().clone();
    let origin = module.origin();
    let raw_pointer = module.image().raw().as_ptr();
    let mut replacement = fixture();
    replacement[file_offset(0x0071_400d)] = 0x80;
    let replacement_path = temp.write("replacement.exe", &replacement);
    fs::rename(replacement_path, &path).unwrap();
    let later = read(&path).unwrap();
    assert_ne!(later.observation().before.inode, observation.before.inode);
    assert_ne!(later.origin(), origin);
    assert_eq!(later.initial_span(FLAG, 1).unwrap().bytes(), &[0x80]);
    // A begin that reopens the observed pathname would now fail or use the
    // wrong owner. Only the original consumed Module supports the zero branch.
    let mut entry = Entry::begin(module).unwrap();
    fs::remove_file(&path).unwrap();
    drop(temp);
    entry.advance().unwrap();
    assert_eq!(entry.module().origin(), origin);
    assert_eq!(entry.module().observation(), &observation);
    assert_eq!(entry.module().image().raw().as_ptr(), raw_pointer);
    assert_eq!(entry.module().image().raw(), bytes);
    assert_eq!(entry.module().initial_span(FLAG, 1).unwrap().bytes(), &[0]);
    assert_eq!(entry.current_span(FLAG, 1).unwrap().bytes(), &[0]);
    assert!(entry.accepts_call(entry.pending_call().unwrap()));
    // Module's reader closed its descriptor after observing it. Entry retains
    // that observation and owning buffers, without claiming an open descriptor.
    assert_eq!(entry.module().observation().before, observation.after);
}
