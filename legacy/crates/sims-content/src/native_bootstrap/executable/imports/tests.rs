use super::*;
use crate::native_bootstrap::executable::read;
use std::{fs, io::Write as _, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

const BASE: u32 = 0x0040_0000;
const OPTIONAL: usize = 0x98;
const RECORDS: usize = 0x178;
const DATA_RAW: usize = 0x1a00;
const DATA_RVA: u32 = 0x1000;
const LOOKUP: u32 = 0x2002; // intentionally unaligned, as in the installed input
const IAT: u32 = 0x7000;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("sims-import-plan-{}-{id}", std::process::id()));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create authored test directory: {error}"),
            }
        }
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).unwrap();
        file.write_all(bytes).unwrap();
        path
    }
}
impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn put16(bytes: &mut [u8], at: usize, value: u16) { bytes[at..at+2].copy_from_slice(&value.to_le_bytes()); }
fn put32(bytes: &mut [u8], at: usize, value: u32) { bytes[at..at+4].copy_from_slice(&value.to_le_bytes()); }
fn raw(rva: u32) -> usize { DATA_RAW + (rva - DATA_RVA) as usize }
fn word(bytes: &mut [u8], rva: u32, value: u32) { put32(bytes, raw(rva), value); }
fn string(bytes: &mut [u8], rva: u32, value: &[u8]) {
    let at = raw(rva);bytes[at..at+value.len()].copy_from_slice(value);
}
fn directory(bytes: &mut [u8], index: usize, rva: u32, size: u32) {
    put32(bytes, OPTIONAL + 96 + index*8, rva);
    put32(bytes, OPTIONAL + 100 + index*8, size);
}
fn descriptor(bytes: &mut [u8], at: u32, lookup: u32, library: u32, iat: u32) {
    for (index,value) in [lookup,0,u32::MAX,library,iat].into_iter().enumerate() {
        word(bytes, at+index as u32*4, value);
    }
}
fn section(bytes: &mut [u8], index: usize, name: &[u8;8], rva: u32,
           virtual_size: u32, raw_size: u32, raw_at: u32, flags: u32) {
    let at = RECORDS + index*40;
    bytes[at..at+40].fill(0);bytes[at..at+8].copy_from_slice(name);
    for (offset,value) in [(8,virtual_size),(12,rva),(16,raw_size),(20,raw_at),(36,flags)] {
        put32(bytes,at+offset,value);
    }
}

// Authored PE32 with sorted .idata/.text/stxt371 spans. Only the entry bytes
// needed by the existing owning Entry profile are actual source observations;
// unrelated filler/data is authored, not protected code, CRT or execution.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; DATA_RAW+0x10000];
    bytes[..2].copy_from_slice(b"MZ");put32(&mut bytes,0x3c,0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes,0x84,0x014c);put16(&mut bytes,0x86,3);
    put16(&mut bytes,0x94,224);put16(&mut bytes,0x96,0x0102);
    put16(&mut bytes,OPTIONAL,0x010b);
    for (offset,value) in [(4,0x1800),(12,0x10000),(16,0x31406e),(20,0x238000),
        (24,0x1000),(28,BASE),(32,0x1000),(36,0x200),(56,0x315000),(60,0x200),
        (72,0x100000),(76,0x1000),(80,0x100000),(84,0x1000),(92,16)] {
        put32(&mut bytes,OPTIONAL+offset,value);
    }
    put16(&mut bytes,OPTIONAL+40,4);put16(&mut bytes,OPTIONAL+48,4);
    put16(&mut bytes,OPTIONAL+68,3);
    section(&mut bytes,0,b".idata\0\0",DATA_RVA,0x10000,0x10000,DATA_RAW as u32,0xc0000040);
    section(&mut bytes,1,b".text\0\0\0",0x238000,0x800,0x600,0x200,0x60000020);
    section(&mut bytes,2,b"stxt371\0",0x313000,0x1800,0x1200,0x800,0xe0000020);
    for (index,byte) in bytes[0x200..0x1a00].iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(29).wrapping_add(0x53);
    }
    let source = concat!(
        "558bec60bb6e407100b80d40710033c98a0885c9740cb8e44071002bc383e805eb0e",
        "51b92b4171008bc12bc303410159c603e98943015168d93f710033c085c974058b4508",
        "eb0050e825fcffff");
    let source: Vec<u8> = source.as_bytes().chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect();
    let entry = 0x800+(0x31406e-0x313000);
    bytes[entry..entry+77].copy_from_slice(&source);
    bytes[0x800+(0x31400d-0x313000)] = 0;
    let jump = 0x800+(0x31412b-0x313000);
    bytes[jump..jump+5].copy_from_slice(&[0xe9,0x34,0x42,0xf2,0xff]);
    directory(&mut bytes,1,0x1200,40);directory(&mut bytes,12,IAT,12);
    descriptor(&mut bytes,0x1200,LOOKUP,0x1120,IAT);
    put16(&mut bytes,raw(0x1100),7);
    string(&mut bytes,0x1102,b"GlobalAlloc\0");string(&mut bytes,0x1120,b"KERNEL32.dll\0");
    for start in [LOOKUP,IAT] { word(&mut bytes,start,0x1100);word(&mut bytes,start+4,0x8000002a); }
    bytes
}

fn plan(bytes: &[u8]) -> ImportPlan {
    let temp = Temp::new();read(&temp.write("input.exe",bytes)).unwrap().begin_initial_imports().unwrap()
}
fn refused(bytes: &[u8], expected: &str) {
    let temp = Temp::new();let path = temp.write("refused.exe",bytes);
    let module = read(&path).expect("authored control must reach import preflight through real PE reader");
    let origin = module.origin();let observation = module.observation().clone();
    let raw_pointer = module.image().raw().as_ptr();let initial_pointer = module.image().initial_bytes().as_ptr();
    let initial = module.image().initial_bytes().to_vec();
    let failure = module.begin_initial_imports().expect_err(expected);
    assert!(failure.error.to_string().contains(expected), "{}",failure.error);
    assert_eq!(failure.module.origin(),origin);assert_eq!(failure.module.observation(),&observation);
    assert_eq!(failure.module.image().raw(),bytes);assert_eq!(failure.module.image().initial_bytes(),initial);
    assert_eq!(failure.module.image().raw().as_ptr(),raw_pointer);
    assert_eq!(failure.module.image().initial_bytes().as_ptr(),initial_pointer);
    fs::remove_file(path).unwrap();assert_eq!(failure.module.initial_span(BASE,2).unwrap().bytes(),b"MZ");
}

#[test]
fn imports_preflight_retains_actual_owner_views_preimages_and_unadvanced_entry() {
    let bytes = fixture();let temp = Temp::new();let path = temp.write("input.exe",&bytes);
    let module = read(&path).unwrap();let origin = module.origin();let observed = module.observation().clone();
    let pointer = module.image().initial_bytes().as_ptr();let initial = module.image().initial_bytes().to_vec();
    let plan = module.begin_initial_imports().unwrap();
    assert_eq!(plan.module().origin(),origin);assert_eq!(plan.module().observation(),&observed);
    assert_eq!(plan.module().image().raw(),bytes);assert_eq!(plan.module().image().initial_bytes().as_ptr(),pointer);
    assert_eq!(plan.entry.current_span(0x71406e,77).unwrap().bytes(),plan.module().initial_span(0x71406e,77).unwrap().bytes());
    assert!(plan.entry.writes().is_empty() && plan.entry.pending_call().is_none());
    assert_eq!(plan.module().image().initial_bytes(),initial);
    let header = plan.module().image().header();
    assert_eq!(plan.entry.current_span(BASE,header.size_of_headers as usize).unwrap().bytes(),
        &initial[..header.size_of_headers as usize]);
    for section in plan.module().image().sections() {
        let length = section.virtual_size.max(section.size_of_raw_data) as usize;
        assert_eq!(plan.entry.current_span(BASE+section.virtual_address,length).unwrap().bytes(),
            plan.module().image().initial_rva(section.virtual_address,length).unwrap());
    }
    assert_eq!((plan.len(),plan.is_empty(),plan.descriptor_count()),(2,false,1));
    assert_eq!(plan.terminators(),[BASE+IAT+8]);assert_eq!(plan.requests().len(),2);
    let first = plan.request(0).unwrap();assert_eq!(first.origin(),origin);
    assert_eq!((first.slot_address(),first.initial_word(),first.descriptor_index()),(BASE+IAT,0x1100,0));
    assert_eq!(first.library(),b"KERNEL32.dll");
    assert_eq!(first.symbol(),&ImportSymbol::Name{hint:7,name:b"GlobalAlloc".to_vec()});
    assert_eq!(plan.request(1).unwrap().symbol(),&ImportSymbol::Ordinal(42));
    assert!(plan.request(2).is_none());
    let descriptor = plan.descriptors().next().unwrap();
    assert_eq!((descriptor.lookup_rva(),descriptor.iat_address()),(LOOKUP,BASE+IAT));
    assert_eq!(descriptor.requests().len(),2);
}

#[test]
fn imports_preflight_accepts_unaligned_iat_and_lookup_as_bytes() {
    let mut bytes = fixture();word(&mut bytes,0x1210,IAT+1);
    word(&mut bytes,IAT,0);word(&mut bytes,IAT+1,0x1100);
    word(&mut bytes,IAT+5,0x8000002a);word(&mut bytes,IAT+9,0);
    directory(&mut bytes,12,IAT+1,12);
    let plan = plan(&bytes);
    assert_eq!(plan.request(0).unwrap().slot_address(),BASE+IAT+1);
    assert_eq!(plan.request(1).unwrap().initial_word(),0x8000002a);
}

#[test]
fn imports_preflight_zero_original_thunk_uses_the_actual_iat_lookup() {
    let mut bytes = fixture();word(&mut bytes,0x1200,0);
    let plan = plan(&bytes);assert_eq!(plan.descriptors().next().unwrap().lookup_rva(),IAT);
    assert_eq!(plan.request(0).unwrap().initial_word(),0x1100);
}

#[test]
fn imports_preflight_retains_an_empty_library_dependency() {
    let mut bytes = fixture();directory(&mut bytes,1,0x1200,60);
    string(&mut bytes,0x1140,b"EMPTY.dll\0");descriptor(&mut bytes,0x1214,0x2500,0x1140,IAT+16);
    directory(&mut bytes,12,IAT,20);
    let plan = plan(&bytes);assert_eq!((plan.len(),plan.descriptor_count()),(2,2));
    let empty = plan.descriptors().nth(1).unwrap();
    assert_eq!(empty.library(),b"EMPTY.dll");assert_eq!(empty.requests().len(),0);
    assert_eq!(plan.terminators(),[BASE+IAT+8,BASE+IAT+16]);
}

#[test]
fn imports_preflight_rejects_duplicate_iat_descriptors_before_any_effect() {
    let mut bytes = fixture();directory(&mut bytes,1,0x1200,60);
    descriptor(&mut bytes,0x1214,LOOKUP,0x1120,IAT);
    refused(&bytes,"overlapping or aliased IAT");
}

#[test]
fn imports_preflight_rejects_partly_overlapping_valid_file_backed_slots() {
    let mut bytes = fixture();directory(&mut bytes,1,0x1200,60);
    put16(&mut bytes,raw(0x1030),7);string(&mut bytes,0x1032,b"GlobalAlloc\0");
    put16(&mut bytes,0x10,8);bytes[0x12..0x1d].copy_from_slice(b"GlobalFree\0");
    word(&mut bytes,LOOKUP,0x1030);word(&mut bytes,LOOKUP+4,0);
    word(&mut bytes,IAT,0x1030);word(&mut bytes,IAT+4,0);
    word(&mut bytes,0x2600,0x10);
    descriptor(&mut bytes,0x1214,0x2600,0x1120,IAT+1);
    refused(&bytes,"overlapping or aliased IAT");
}

#[test]
fn imports_preflight_rejects_a_reused_terminator_even_without_new_symbols() {
    let mut bytes = fixture();directory(&mut bytes,1,0x1200,60);
    descriptor(&mut bytes,0x1214,0x2500,0x1120,IAT+8);
    refused(&bytes,"overlapping or aliased IAT");
}

#[test]
fn imports_preflight_rejects_stale_iat_words_and_bound_timestamps() {
    let mut bytes = fixture();word(&mut bytes,IAT,0x12345678);
    refused(&bytes,"initial IAT differs");
    let mut bytes = fixture();word(&mut bytes,0x1204,1);
    refused(&bytes,"bound import timestamp");
}

#[test]
fn imports_preflight_rejects_opaque_bound_delay_and_forwarder_profiles() {
    for index in [11,13] {
        let mut bytes = fixture();directory(&mut bytes,index,0x1500,8);
        refused(&bytes,"bound or delay import directories");
    }
    let mut bytes = fixture();word(&mut bytes,0x1208,1);
    refused(&bytes,"forwarder chains require another");
}

#[test]
fn imports_preflight_requires_declared_writable_section_storage_not_headers() {
    let mut bytes = fixture();put32(&mut bytes,RECORDS+36,0x40000040);
    refused(&bytes,"readable writable declared section");
    let mut bytes = fixture();word(&mut bytes,0x1210,0x40);
    put32(&mut bytes,0x40,0x1100);put32(&mut bytes,0x44,0x8000002a);put32(&mut bytes,0x48,0);
    directory(&mut bytes,12,0x40,12);
    refused(&bytes,"readable writable declared section");
}

#[test]
fn imports_preflight_checks_declared_iat_coverage_including_terminators() {
    for (start,size) in [(IAT+4,12),(IAT,8)] {
        let mut bytes = fixture();directory(&mut bytes,12,start,size);
        refused(&bytes,"outside declared IAT directory");
    }
    let mut bytes = fixture();directory(&mut bytes,12,0,0);
    assert_eq!(plan(&bytes).len(),2);
}

#[test]
fn imports_preflight_refuses_protected_entry_flag_alias_even_for_zero_imports() {
    let mut bytes = fixture();word(&mut bytes,0x1200,0x2500);
    word(&mut bytes,0x1210,0x31400d);
    directory(&mut bytes,12,0x31400d,4);
    let at = 0x800+(0x31400d-0x313000);bytes[at..at+4].fill(0);
    refused(&bytes,"IAT overlaps protected entry-image source");
}

#[test]
fn imports_preflight_preserves_owner_when_the_entry_profile_rejects_after_validation() {
    let mut bytes = fixture();bytes[0x800+(0x31406e-0x313000)] = 0x90;
    refused(&bytes,"initial entry");
}

#[test]
fn imports_preflight_bounds_total_requests_before_building_an_entry_image() {
    let mut bytes = fixture();let count = MAX_IMPORTS+1;
    for index in 0..count {
        word(&mut bytes,LOOKUP+index as u32*4,0x8000002a);
        word(&mut bytes,IAT+index as u32*4,0x8000002a);
    }
    word(&mut bytes,LOOKUP+count as u32*4,0);word(&mut bytes,IAT+count as u32*4,0);
    directory(&mut bytes,12,IAT,(count as u32+1)*4);
    refused(&bytes,"normal import count exceeds executable profile");
}

#[test]
fn imports_preflight_no_directory_means_no_unclaimed_iat_or_bound_image_claim() {
    let mut bytes = fixture();directory(&mut bytes,1,0,0);
    refused(&bytes,"IAT directory has no normal import descriptors");
    directory(&mut bytes,12,0,0);
    let plan = plan(&bytes);assert!(plan.is_empty());assert_eq!(plan.descriptor_count(),0);
    assert!(plan.entry.writes().is_empty() && plan.entry.pending_call().is_none());
}

#[test]
fn imports_preflight_distinguishes_identical_file_reads_and_retains_unlinked_bytes() {
    let bytes = fixture();let temp = Temp::new();let path = temp.write("input.exe",&bytes);
    let first = read(&path).unwrap().begin_initial_imports().unwrap();
    let second = read(&path).unwrap().begin_initial_imports().unwrap();
    assert_ne!(first.request(0).unwrap().origin(),second.request(0).unwrap().origin());
    let replacement = temp.write("replacement.exe",&bytes);fs::rename(replacement,&path).unwrap();
    fs::remove_file(path).unwrap();
    assert_eq!(first.module().image().raw(),bytes);
    assert_eq!(first.request(0).unwrap().library(),b"KERNEL32.dll");
    assert_eq!(first.descriptors().next().unwrap().library(),b"KERNEL32.dll");
}

#[test]
#[ignore = "parent-only fixed installed EXE input, no import binding or execution"]
fn imports_preflight_parent_installed_thirteen_slots_remain_unbound() {
    let path = PathBuf::from(std::env::var_os("SIMS_PARENT_INSTALLED_EXECUTABLE").unwrap());
    let module = read(&path).unwrap();
    let hex = |bytes:&[u8]| bytes.iter().map(|b|format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(&module.observation().sha256),"32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58");
    assert_eq!(module.observation().bytes_read,4_422_663);
    let origin = module.origin();let plan = module.begin_initial_imports().unwrap();
    assert_eq!((plan.len(),plan.descriptor_count(),plan.terminators().len()),(13,9,9));
    assert_eq!(plan.request(0).unwrap().slot_address(),0x7142c8);
    assert_eq!(plan.request(0).unwrap().initial_word(),0x314320);
    assert!(plan.requests().all(|request| request.origin()==origin));
    assert!(plan.entry.writes().is_empty() && plan.entry.pending_call().is_none());
    assert!(plan.descriptors().any(|descriptor| descriptor.lookup_rva()%4!=0));
    for request in plan.requests() {
        assert_eq!(plan.entry.current_span(request.slot_address(),4).unwrap().bytes(),
            request.initial_word().to_le_bytes());
    }
    println!("{}",serde_json::json!({"status":"INSTALLED_NORMAL_IMPORT_PREFLIGHT_ONLY",
        "imports":13,"libraries":9,"import_words_installed":0,"entry_stores":0,
        "imports_bound":false,"cpu_or_stack_executed":false,"driver_executed":false,
        "crt_initialized":false,"constructor_entry_state_qualified":false,"all_alpha_gates":false}));
}

