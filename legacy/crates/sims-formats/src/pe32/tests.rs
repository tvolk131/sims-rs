//! Authored fixtures only. Parent owns all compilation and execution.
use super::{Image, ImportSymbol, MAX_FILE_BYTES, MAX_IMAGE_BYTES};

const COFF: usize = 0x84;
const OPTIONAL: usize = 0x98;
const SECTION: usize = 0x178;
const BASE: u32 = 0x0040_0000;

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn minimal() -> Vec<u8> {
    let mut bytes = vec![0; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, COFF, 0x014c);
    put16(&mut bytes, COFF + 2, 1);
    put16(&mut bytes, COFF + 16, 224);
    put16(&mut bytes, COFF + 18, 0x0102);
    put16(&mut bytes, OPTIONAL, 0x010b);
    put32(&mut bytes, OPTIONAL + 16, 0x1000);
    put32(&mut bytes, OPTIONAL + 28, BASE);
    put32(&mut bytes, OPTIONAL + 32, 0x1000);
    put32(&mut bytes, OPTIONAL + 36, 0x200);
    put32(&mut bytes, OPTIONAL + 56, 0x2000);
    put32(&mut bytes, OPTIONAL + 60, 0x200);
    put32(&mut bytes, OPTIONAL + 92, 16);
    bytes[SECTION..SECTION + 8].copy_from_slice(b".text\0\0\0");
    put32(&mut bytes, SECTION + 8, 0x300);
    put32(&mut bytes, SECTION + 12, 0x1000);
    put32(&mut bytes, SECTION + 16, 0x200);
    put32(&mut bytes, SECTION + 20, 0x200);
    put32(&mut bytes, SECTION + 36, 0x6000_0020);
    for (index, byte) in bytes[0x200..].iter_mut().enumerate() {
        *byte = ((index % 251) + 1) as u8;
    }
    bytes
}

fn two_sections() -> Vec<u8> {
    let mut bytes = minimal();
    bytes.resize(0x600, 0x5a);
    put16(&mut bytes, COFF + 2, 2);
    put32(&mut bytes, OPTIONAL + 56, 0x3000);
    let second = SECTION + 40;
    bytes[second..second + 40].fill(0);
    bytes[second..second + 8].copy_from_slice(b".data\0\0\0");
    put32(&mut bytes, second + 8, 0x300);
    put32(&mut bytes, second + 12, 0x2000);
    put32(&mut bytes, second + 16, 0x200);
    put32(&mut bytes, second + 20, 0x400);
    put32(&mut bytes, second + 36, 0xc000_0040);
    bytes
}

fn directory(bytes: &mut [u8], index: usize, address: u32, size: u32) {
    let offset = OPTIONAL + 96 + index * 8;
    put32(bytes, offset, address);
    put32(bytes, offset + 4, size);
}

fn file_offset(rva: u32) -> usize {
    0x200 + (rva - 0x1000) as usize
}

fn rva32(bytes: &mut [u8], rva: u32, value: u32) {
    put32(bytes, file_offset(rva), value);
}

fn rva_bytes(bytes: &mut [u8], rva: u32, value: &[u8]) {
    let offset = file_offset(rva);
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

fn imports_fixture() -> Vec<u8> {
    let mut bytes = minimal();
    bytes.resize(0x800, 0);
    bytes[0x200..].fill(0);
    put32(&mut bytes, SECTION + 8, 0x700);
    put32(&mut bytes, SECTION + 16, 0x600);
    directory(&mut bytes, 1, 0x1000, 40);
    rva32(&mut bytes, 0x1000, 0x1100); // OriginalFirstThunk
    rva32(&mut bytes, 0x100c, 0x1140); // DLL name
    rva32(&mut bytes, 0x1010, 0x1120); // FirstThunk
    for start in [0x1100, 0x1120] {
        rva32(&mut bytes, start, 0x1160);
        rva32(&mut bytes, start + 4, 0x8000_002a);
    }
    rva_bytes(&mut bytes, 0x1140, b"KERNEL32.dll\0");
    put16(&mut bytes, file_offset(0x1160), 7);
    rva_bytes(&mut bytes, 0x1162, b"CreateFileA\0");
    bytes
}

#[test]
fn consumes_and_retains_exact_initial_file_bytes() {
    let bytes = minimal();
    let original_address = bytes.as_ptr();
    let expected = bytes.clone();
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.raw().as_ptr(), original_address);
    assert_eq!(image.raw(), expected);
    assert_eq!(image.initial_bytes().len(), 0x2000);
    assert_eq!(image.header().image_base, BASE);
    assert_eq!(image.header().size_of_headers, 0x200);
    assert_eq!(image.header().size_of_image, 0x2000);
    assert_eq!(image.header().address_of_entry_point, 0x1000);
    assert_eq!(image.header().file_alignment, 0x200);
    assert_eq!(image.header().section_alignment, 0x1000);
    assert_eq!(image.sections().len(), 1);
    assert_eq!(image.sections()[0].name, *b".text\0\0\0");
    assert_eq!(image.sections()[0].characteristics, 0x6000_0020);
    assert_eq!(image.data_directories().len(), 16);
    assert!(image.imports().is_empty());
    assert!(format!("{image:?}").len() < 512);
}

#[test]
fn headers_sections_and_preferred_va_queries_match_exact_bytes() {
    let bytes = minimal();
    let image = Image::parse(bytes.clone()).unwrap();
    assert_eq!(image.initial_rva(0, 0x200).unwrap(), &bytes[..0x200]);
    assert_eq!(image.file_backed_rva(0, 0x200).unwrap(), &bytes[..0x200]);
    assert_eq!(image.initial_va(BASE, 2).unwrap(), b"MZ");
    assert_eq!(image.initial_rva(0x1000, 0x200).unwrap(), &bytes[0x200..]);
    assert_eq!(image.file_backed_rva(0x1000, 0x200).unwrap(), &bytes[0x200..]);
    assert_eq!(image.initial_va(BASE + 0x1000, 0x200).unwrap(), &bytes[0x200..]);
    assert_eq!(image.initial_rva(0x12ff, 1).unwrap(), &[0]);
}

#[test]
fn raw_padding_and_zero_virtual_size_keep_every_declared_raw_byte() {
    for virtual_size in [0, 0x80, 0x180] {
        let mut bytes = minimal();
        put32(&mut bytes, SECTION + 8, virtual_size);
        let expected = bytes[0x200..0x400].to_vec();
        let image = Image::parse(bytes).unwrap();
        assert_eq!(image.initial_rva(0x1000, 0x200).unwrap(), expected);
        assert_eq!(image.file_backed_rva(0x11ff, 1).unwrap(), &expected[0x1ff..]);
        assert!(image.initial_rva(0x1200, 1).is_err());
    }
}

#[test]
fn bss_is_logical_zero_and_never_file_backed() {
    let image = Image::parse(minimal()).unwrap();
    assert_eq!(image.initial_rva(0x1200, 0x100).unwrap(), &[0; 0x100]);
    assert!(image.file_backed_rva(0x1200, 1).is_err());
    assert!(image.file_backed_rva(0x11ff, 2).is_err());
    let mut bytes = minimal();
    put32(&mut bytes, SECTION + 16, 0);
    put32(&mut bytes, SECTION + 20, 0);
    bytes.truncate(0x200);
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.initial_rva(0x1000, 0x300).unwrap(), &[0; 0x300]);
    assert!(image.file_backed_rva(0x1000, 1).is_err());
}

#[test]
fn gaps_reserved_tails_and_all_crossings_are_rejected() {
    let image = Image::parse(two_sections()).unwrap();
    assert!(image.initial_bytes()[0x200..0x1000].iter().all(|&byte| byte == 0));
    assert!(image.initial_bytes()[0x1300..0x2000].iter().all(|&byte| byte == 0));
    for (rva, len) in [(0x200, 1), (0x800, 1), (0x1300, 1), (0x1fff, 2),
        (0x1ff, 2), (0x12ff, 2), (0x2300, 1), (0x3000, 1)]
    {
        assert!(image.initial_rva(rva, len).is_err(), "{rva:x}/{len}");
        assert!(image.file_backed_rva(rva, len).is_err(), "{rva:x}/{len}");
    }
    let mut adjacent = two_sections();
    put32(&mut adjacent, SECTION + 8, 0x1000);
    let adjacent = Image::parse(adjacent).unwrap();
    assert!(adjacent.initial_rva(0x1fff, 1).is_ok());
    assert!(adjacent.initial_rva(0x2000, 1).is_ok());
    assert!(adjacent.initial_rva(0x1fff, 2).is_err());
    let mut trailing_gap = minimal();
    put32(&mut trailing_gap, OPTIONAL + 56, 0x4000);
    let image = Image::parse(trailing_gap).unwrap();
    assert_eq!(image.initial_bytes().len(), 0x4000);
    assert!(image.initial_rva(0x3000, 1).is_err());
}

#[test]
fn zero_overflow_and_below_base_queries_fail() {
    let image = Image::parse(minimal()).unwrap();
    for (rva, len) in [(0, 0), (0x1000, 0), (u32::MAX, 1), (u32::MAX, 2),
        (0x1000, usize::MAX), (0, usize::MAX)]
    {
        assert!(image.initial_rva(rva, len).is_err());
        assert!(image.file_backed_rva(rva, len).is_err());
    }
    for (va, len) in [(BASE - 1, 1), (BASE, 0), (u32::MAX, 2),
        (BASE + 0x1000, usize::MAX), (BASE + 0x1300, 1)]
    {
        assert!(image.initial_va(va, len).is_err());
    }
}

#[test]
fn every_truncated_prefix_is_rejected_without_panicking() {
    let bytes = minimal();
    for len in 0..bytes.len() {
        assert!(Image::parse(bytes[..len].to_vec()).is_err(), "prefix {len}");
    }
}

#[test]
fn rejects_wrong_signatures_machine_magic_and_image_flags() {
    for (offset, value) in [(0, 0), (0x80, 0), (COFF, 0x8664),
        (OPTIONAL, 0x020b), (OPTIONAL, 0x0107), (COFF + 18, 0),
        (COFF + 18, 0x8182)]
    {
        let mut bytes = minimal();
        put16(&mut bytes, offset, value);
        assert!(Image::parse(bytes).is_err());
    }
}

#[test]
fn rejects_inconsistent_or_uncovered_header_tables() {
    for count in [0, 97, u16::MAX] {
        let mut bytes = minimal();
        put16(&mut bytes, COFF + 2, count);
        assert!(Image::parse(bytes).is_err());
    }
    for optional_size in [0, 95, 96, 223, u16::MAX] {
        let mut bytes = minimal();
        put16(&mut bytes, COFF + 16, optional_size);
        assert!(Image::parse(bytes).is_err());
    }
    for count in [17, u32::MAX] {
        let mut bytes = minimal();
        put32(&mut bytes, OPTIONAL + 92, count);
        assert!(Image::parse(bytes).is_err());
    }
    let mut bytes = minimal();
    put16(&mut bytes, COFF + 2, 4); // fourth table record exceeds headers
    assert!(Image::parse(bytes).is_err());
    for pe in [0, 0x3c, 0xffff_fff0, u32::MAX] {
        let mut bytes = minimal();
        put32(&mut bytes, 0x3c, pe);
        assert!(Image::parse(bytes).is_err());
    }
}

#[test]
fn accepts_a_complete_optional_header_without_directories() {
    let mut bytes = minimal();
    let section = bytes[SECTION..SECTION + 40].to_vec();
    put16(&mut bytes, COFF + 16, 96);
    put32(&mut bytes, OPTIONAL + 92, 0);
    bytes[OPTIONAL + 96..OPTIONAL + 96 + 40].copy_from_slice(&section);
    let image = Image::parse(bytes).unwrap();
    assert!(image.data_directories().is_empty());
    assert!(image.imports().is_empty());
    assert_eq!(image.sections().len(), 1);
}

#[test]
fn optional_header_padding_is_not_adopted_as_extra_directories() {
    for count in [0, 1, 15, 16] {
        let mut bytes = minimal();
        put32(&mut bytes, OPTIONAL + 92, count);
        let image = Image::parse(bytes).unwrap();
        assert_eq!(image.data_directories().len(), count as usize);
        assert_eq!(image.sections().len(), 1);
    }
}

#[test]
fn rejects_unsupported_alignment_and_image_extents() {
    for (field, value) in [(32, 0), (32, 2048), (32, 6144), (36, 0),
        (36, 256), (36, 513), (36, 131_072), (36, 8192), (28, BASE + 1),
        (56, 0), (56, 0x1000), (56, 0x2100), (60, 0), (60, 0x201),
        (60, 0x4000), (56, MAX_IMAGE_BYTES as u32 + 0x1000)]
    {
        let mut bytes = minimal();
        put32(&mut bytes, OPTIONAL + field, value);
        assert!(Image::parse(bytes).is_err(), "optional field {field}: {value:x}");
    }
    let mut bytes = minimal();
    put32(&mut bytes, OPTIONAL + 28, 0xffff_0000);
    put32(&mut bytes, OPTIONAL + 56, 0x1_0000);
    assert!(Image::parse(bytes).is_err());
    let mut bytes = minimal();
    put32(&mut bytes, OPTIONAL + 32, 0x8000_0000);
    put32(&mut bytes, OPTIONAL + 56, 0x8000_0000);
    assert!(Image::parse(bytes).is_err());
}

#[test]
fn accepts_supported_alignment_extremes_and_high_preferred_bases() {
    let mut bytes = minimal();
    put32(&mut bytes, OPTIONAL + 28, 0xffff_0000);
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.initial_va(0xffff_1000, 2).unwrap(), &[1, 2]);

    let mut bytes = minimal();
    let original_raw = bytes[0x200..].to_vec();
    bytes.resize(0x20000, 0);
    bytes[0x10000..0x10200].copy_from_slice(&original_raw);
    for (field, value) in [(16, 0x10000), (32, 0x10000), (36, 0x10000),
        (56, 0x20000), (60, 0x10000)]
    {
        put32(&mut bytes, OPTIONAL + field, value);
    }
    put32(&mut bytes, SECTION + 12, 0x10000);
    put32(&mut bytes, SECTION + 16, 0x10000);
    put32(&mut bytes, SECTION + 20, 0x10000);
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.initial_rva(0x10000, 0x200).unwrap(), original_raw);
    assert!(image.file_backed_rva(0x1ffff, 1).is_ok());
    // Even physically adjacent headers and sections remain separate spans.
    assert!(image.initial_rva(0xffff, 2).is_err());
    assert!(image.file_backed_rva(0xffff, 2).is_err());
}

#[test]
fn file_size_limit_precedes_format_or_mapping_allocation() {
    assert_eq!(MAX_FILE_BYTES, 64 * 1024 * 1024);
    assert_eq!(MAX_IMAGE_BYTES, MAX_FILE_BYTES);
    let bytes = vec![0; MAX_FILE_BYTES + 1];
    let error = Image::parse(bytes).unwrap_err();
    assert!(error.message.contains("file exceeds"));
}

#[test]
fn rejects_empty_sections_invalid_raw_spans_and_wrapping_extents() {
    let mut empty = minimal();
    put32(&mut empty, SECTION + 8, 0);
    put32(&mut empty, SECTION + 16, 0);
    put32(&mut empty, SECTION + 20, 0);
    assert!(Image::parse(empty).is_err());
    let mut empty_raw = minimal();
    put32(&mut empty_raw, SECTION + 16, 0);
    assert!(Image::parse(empty_raw).is_err());
    for (field, value) in [(8, u32::MAX), (12, 0), (12, 0x1001),
        (12, 0xffff_f000), (16, 1), (16, 0x400), (16, 0xffff_fe00),
        (20, 0), (20, 0x201), (20, 0x400), (20, 0xffff_fe00)]
    {
        let mut bytes = minimal();
        put32(&mut bytes, SECTION + field, value);
        assert!(Image::parse(bytes).is_err(), "section field {field}: {value:x}");
    }
}

#[test]
fn rejects_virtual_raw_and_reserved_extent_overlaps() {
    for second_rva in [0, 0x1000, 0x1001] {
        let mut bytes = two_sections();
        put32(&mut bytes, SECTION + 40 + 12, second_rva);
        assert!(Image::parse(bytes).is_err());
    }
    let mut bytes = two_sections();
    put32(&mut bytes, SECTION + 8, 0x1001);
    assert!(Image::parse(bytes).is_err());
    let mut bytes = two_sections();
    put32(&mut bytes, SECTION + 40 + 20, 0x200);
    assert!(Image::parse(bytes).is_err());
    let mut bytes = two_sections();
    put32(&mut bytes, SECTION + 12, 0x2000);
    put32(&mut bytes, SECTION + 40 + 12, 0x1000);
    assert!(Image::parse(bytes).is_err());
    let mut bytes = minimal();
    put32(&mut bytes, OPTIONAL + 16, 0x1300);
    assert!(Image::parse(bytes).is_err());
}

#[test]
fn raw_file_spans_need_not_follow_virtual_order() {
    let mut bytes = two_sections();
    put32(&mut bytes, SECTION + 20, 0x400);
    put32(&mut bytes, SECTION + 40 + 20, 0x200);
    let first = bytes[0x400..0x600].to_vec();
    let second = bytes[0x200..0x400].to_vec();
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.initial_rva(0x1000, 0x200).unwrap(), first);
    assert_eq!(image.file_backed_rva(0x2000, 0x200).unwrap(), second);
}

#[test]
fn directories_require_coherent_bounded_declared_spans() {
    for (rva, size) in [(0, 1), (0x1000, 0), (0x800, 1), (0x1300, 1),
        (0x12ff, 2), (0x1ff, 2), (u32::MAX, 2), (0x1000, u32::MAX)]
    {
        let mut bytes = minimal();
        directory(&mut bytes, 2, rva, size);
        assert!(Image::parse(bytes).is_err(), "directory {rva:x}/{size:x}");
    }
    for (rva, size) in [(0x100, 16), (0x1000, 0x300), (0x1200, 0x100)] {
        let mut bytes = minimal();
        directory(&mut bytes, 2, rva, size);
        let image = Image::parse(bytes).unwrap();
        assert_eq!(image.data_directories()[2].virtual_address, rva);
        assert_eq!(image.data_directories()[2].size, size);
    }
}

#[test]
fn certificate_directory_is_a_file_offset_not_an_rva() {
    let mut bytes = minimal();
    bytes.resize(0x420, 0xa5);
    directory(&mut bytes, 4, 0x400, 0x20); // overlay, not a declared RVA
    let image = Image::parse(bytes).unwrap();
    assert_eq!(&image.raw()[0x400..0x420], &[0xa5; 0x20]);
    assert!(image.initial_rva(0x400, 0x20).is_err());
    assert!(image.file_backed_rva(0x400, 0x20).is_err());
    for (offset, size) in [(0x1000, 16), (0x400, 0x21), (0x401, 8),
        (0xffff_fff8, 16), (0, 16), (0x400, 0)]
    {
        let mut bytes = image.raw().to_vec();
        directory(&mut bytes, 4, offset, size);
        assert!(Image::parse(bytes).is_err());
    }
}

#[test]
fn named_and_ordinal_imports_retain_initial_slot_metadata() {
    let bytes = imports_fixture();
    let expected = bytes.clone();
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.raw(), expected);
    let imports = image.imports();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].dll, b"KERNEL32.dll");
    assert_eq!(imports[0].iat_va, BASE + 0x1120);
    assert_eq!(imports[0].symbol, ImportSymbol::Name {
        hint: 7, name: b"CreateFileA".to_vec(),
    });
    assert_eq!(imports[1].dll, b"KERNEL32.dll");
    assert_eq!(imports[1].iat_va, BASE + 0x1124);
    assert_eq!(imports[1].symbol, ImportSymbol::Ordinal(42));
    assert_eq!(image.initial_va(imports[0].iat_va, 4).unwrap(), &0x1160u32.to_le_bytes());
}

#[test]
fn import_metadata_preserves_multiple_dlls_and_descriptor_order() {
    let mut bytes = imports_fixture();
    directory(&mut bytes, 1, 0x1000, 60);
    rva32(&mut bytes, 0x1014, 0x1540);
    rva32(&mut bytes, 0x1020, 0x1520);
    rva32(&mut bytes, 0x1024, 0x1560);
    rva_bytes(&mut bytes, 0x1520, b"USER32.dll\0");
    rva32(&mut bytes, 0x1540, 0x8000_0009);
    rva32(&mut bytes, 0x1560, 0x8000_0009);
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.imports().len(), 3);
    assert_eq!(image.imports()[0].dll, b"KERNEL32.dll");
    assert_eq!(image.imports()[2].dll, b"USER32.dll");
    assert_eq!(image.imports()[2].iat_va, BASE + 0x1560);
    assert_eq!(image.imports()[2].symbol, ImportSymbol::Ordinal(9));
}

#[test]
fn import_names_are_raw_bytes_without_encoding_authority() {
    let mut bytes = imports_fixture();
    rva_bytes(&mut bytes, 0x1140, b"\xff.dll\0");
    rva_bytes(&mut bytes, 0x1162, b"\x80name\0");
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.imports()[0].dll, b"\xff.dll");
    assert_eq!(image.imports()[0].symbol, ImportSymbol::Name {
        hint: 7, name: b"\x80name".to_vec(),
    });
}

#[test]
fn original_first_thunk_supplies_names_for_bound_iat_values() {
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1004, 0x1234_5678);
    rva32(&mut bytes, 0x1120, 0x7654_3210);
    rva32(&mut bytes, 0x1124, 0x7698_7654);
    let image = Image::parse(bytes).unwrap();
    assert!(matches!(&image.imports()[0].symbol, ImportSymbol::Name { name, .. }
        if name == b"CreateFileA"));
    assert_eq!(image.imports()[1].symbol, ImportSymbol::Ordinal(42));
    assert_eq!(image.initial_rva(0x1120, 4).unwrap(), &0x7654_3210u32.to_le_bytes());
}

#[test]
fn iat_lookup_fallback_requires_an_unbound_timestamp() {
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1000, 0);
    assert_eq!(Image::parse(bytes.clone()).unwrap().imports().len(), 2);
    for timestamp in [1, u32::MAX] {
        rva32(&mut bytes, 0x1004, timestamp);
        assert!(Image::parse(bytes.clone()).is_err());
    }
}

#[test]
fn import_descriptors_must_terminate_within_declared_file_span() {
    for size in [1, 19, 20, 39] {
        let mut bytes = imports_fixture();
        directory(&mut bytes, 1, 0x1000, size);
        assert!(Image::parse(bytes).is_err());
    }
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1018, 1); // a partial-zero descriptor is not a terminator
    assert!(Image::parse(bytes).is_err());
    let mut bytes = imports_fixture();
    directory(&mut bytes, 1, 0x15f0, 40); // includes implicit BSS bytes
    assert!(Image::parse(bytes).is_err());
    let mut bytes = imports_fixture();
    directory(&mut bytes, 1, 0x1600, 20); // all-zero BSS is not a file terminator
    assert!(Image::parse(bytes).is_err());
}

#[test]
fn lookup_and_iat_terminators_must_be_file_backed_and_consistent() {
    for (descriptor_field, value) in [(0, 0x15fc), (16, 0x15fc)] {
        let mut bytes = imports_fixture();
        rva32(&mut bytes, 0x1000 + descriptor_field, value);
        rva32(&mut bytes, 0x15fc, 0x8000_0001);
        assert!(Image::parse(bytes).is_err());
    }
    for (slot, value) in [(0x1120, 0), (0x1128, 1)] {
        let mut bytes = imports_fixture();
        rva32(&mut bytes, slot, value);
        assert!(Image::parse(bytes).is_err());
    }
}

#[test]
fn import_pointers_names_and_hints_cannot_use_bss_or_unmapped_bytes() {
    for (field, value) in [(0, 0x1600), (0, 0xffff_fffc), (0, 0x1101),
        (12, 0), (12, 0x1600), (12, u32::MAX), (16, 0),
        (16, 0x1600), (16, 0x15fe), (16, 0xffff_fffc)]
    {
        let mut bytes = imports_fixture();
        rva32(&mut bytes, 0x1000 + field, value);
        assert!(Image::parse(bytes).is_err());
    }
    for name_rva in [0x1600, 0x15ff, 0x700, 0x7fff_ffff] {
        let mut bytes = imports_fixture();
        rva32(&mut bytes, 0x1100, name_rva);
        assert!(Image::parse(bytes).is_err());
    }
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x100c, 0x15f0);
    rva_bytes(&mut bytes, 0x15f0, &[b'X'; 16]);
    assert!(Image::parse(bytes).is_err());
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1100, 0x15fc);
    rva_bytes(&mut bytes, 0x15fc, &[7, 0, b'X', b'X']);
    assert!(Image::parse(bytes).is_err());
    for empty_rva in [0x1140, 0x1162] {
        let mut bytes = imports_fixture();
        rva_bytes(&mut bytes, empty_rva, &[0]);
        assert!(Image::parse(bytes).is_err());
    }
}

#[test]
fn unaligned_lookup_and_iat_arrays_preserve_symbols_for_all_residues() {
    for lookup_delta in 0..4 {
        for iat_delta in 0..4 {
            let mut bytes = imports_fixture();
            let lookup = bytes[file_offset(0x1100)..file_offset(0x1100) + 12].to_vec();
            let iat = bytes[file_offset(0x1120)..file_offset(0x1120) + 12].to_vec();
            let lookup_rva = 0x1100 + lookup_delta;
            let iat_rva = 0x1120 + iat_delta;
            rva_bytes(&mut bytes, lookup_rva, &lookup);
            rva_bytes(&mut bytes, iat_rva, &iat);
            rva32(&mut bytes, 0x1000, lookup_rva);
            rva32(&mut bytes, 0x1010, iat_rva);
            let image = Image::parse(bytes).unwrap();
            assert_eq!(image.imports().len(), 2);
            assert_eq!(image.imports()[0].dll, b"KERNEL32.dll");
            assert_eq!(image.imports()[0].symbol, ImportSymbol::Name { hint: 7, name: b"CreateFileA".to_vec() });
            assert_eq!(image.imports()[1].symbol, ImportSymbol::Ordinal(42));
            assert_eq!(image.imports()[0].iat_va, BASE + iat_rva);
            assert_eq!(image.imports()[1].iat_va, BASE + iat_rva + 4);
            assert_eq!(image.file_backed_rva(lookup_rva, 12).unwrap(), lookup);
            assert_eq!(image.file_backed_rva(iat_rva, 12).unwrap(), iat);
        }
    }
}

#[test]
fn unaligned_fallback_and_bound_iat_keep_the_existing_name_authority() {
    for delta in 1..4 {
        let mut bytes = imports_fixture();
        let table = bytes[file_offset(0x1120)..file_offset(0x1120) + 12].to_vec();
        rva_bytes(&mut bytes, 0x1120 + delta, &table);
        rva32(&mut bytes, 0x1000, 0);
        rva32(&mut bytes, 0x1010, 0x1120 + delta);
        assert_eq!(Image::parse(bytes.clone()).unwrap().imports().len(), 2);
        rva32(&mut bytes, 0x1004, 9);
        assert!(Image::parse(bytes.clone()).is_err(), "bound fallback must stay unsupported");
        rva_bytes(&mut bytes, 0x1100 + delta, &table);
        rva32(&mut bytes, 0x1000, 0x1100 + delta);
        rva32(&mut bytes, 0x1120 + delta, 0x77aa_1111);
        rva32(&mut bytes, 0x1124 + delta, 0x77aa_2222);
        let image = Image::parse(bytes).unwrap();
        assert_eq!(image.imports()[0].symbol, ImportSymbol::Name { hint: 7, name: b"CreateFileA".to_vec() });
        assert_eq!(image.imports()[1].symbol, ImportSymbol::Ordinal(42));
    }
}

#[test]
fn unaligned_tables_cannot_use_partial_dwords_or_implicit_terminators() {
    for remaining in 1..4 {
        for field in [0, 16] {
            let mut bytes = imports_fixture();
            rva32(&mut bytes, 0x1000 + field, 0x1600 - remaining);
            assert!(Image::parse(bytes).is_err());
            let mut bytes = imports_fixture();
            let start = 0x1600 - 4 - remaining;
            rva32(&mut bytes, start, 0x8000_002a);
            rva32(&mut bytes, 0x1000 + field, start);
            assert!(Image::parse(bytes).is_err(), "partial zero bytes cannot terminate a table");
        }
    }
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1000, 0x15fc);
    rva32(&mut bytes, 0x1010, 0x15fc);
    assert!(Image::parse(bytes).unwrap().imports().is_empty(), "complete final DWORD remains admissible");
}

#[test]
fn metadata_names_remain_lookup_driven_for_shifted_nonzero_iat_bytes() {
    // Former alignment-only negative (field16, 0x1121). The reader preserves
    // raw bytes and reports lookup names; it never resolves these IAT values.
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1010, 0x1121);
    let expected = bytes[file_offset(0x1121)..file_offset(0x1121) + 4].to_vec();
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.imports()[0].iat_va, BASE + 0x1121);
    assert_eq!(image.imports()[0].symbol, ImportSymbol::Name { hint: 7, name: b"CreateFileA".to_vec() });
    assert_eq!(image.file_backed_rva(0x1121, 4).unwrap(), expected);
}

#[test]
fn ordinal_reserved_bits_are_rejected_but_all_sixteen_ordinal_bits_are_kept() {
    for thunk in [0x8001_002a, 0xc000_002a, 0xffff_ffff] {
        let mut bytes = imports_fixture();
        rva32(&mut bytes, 0x1104, thunk);
        assert!(Image::parse(bytes).is_err());
    }
    let mut bytes = imports_fixture();
    rva32(&mut bytes, 0x1104, 0x8000_ffff);
    let image = Image::parse(bytes).unwrap();
    assert_eq!(image.imports()[1].symbol, ImportSymbol::Ordinal(u16::MAX));
}

#[test]
fn import_names_have_an_explicit_length_bound() {
    for length in [4096, 4097] {
        let mut bytes = imports_fixture();
        bytes.resize(0x1600, 0);
        put32(&mut bytes, SECTION + 8, 0x1400);
        put32(&mut bytes, SECTION + 16, 0x1400);
        put32(&mut bytes, OPTIONAL + 56, 0x3000);
        rva32(&mut bytes, 0x100c, 0x1200);
        rva_bytes(&mut bytes, 0x1200, &vec![b'A'; length]);
        rva_bytes(&mut bytes, 0x1200 + length as u32, &[0]);
        let parsed = Image::parse(bytes);
        if length == 4096 {
            assert_eq!(parsed.unwrap().imports()[0].dll.len(), length);
        } else {
            assert!(parsed.is_err());
        }
    }
}

fn descriptor_count_fixture(count: usize) -> Vec<u8> {
    let mut bytes = imports_fixture();
    bytes.resize(0x15200, 0);
    bytes[0x200..].fill(0);
    put32(&mut bytes, SECTION + 8, 0x15000);
    put32(&mut bytes, SECTION + 16, 0x15000);
    put32(&mut bytes, OPTIONAL + 56, 0x16000);
    directory(&mut bytes, 1, 0x1000, ((count + 1) * 20) as u32);
    for index in 0..count {
        let rva = 0x1000 + (index * 20) as u32;
        rva32(&mut bytes, rva, 0x15100);
        rva32(&mut bytes, rva + 12, 0x15110);
        rva32(&mut bytes, rva + 16, 0x15120);
    }
    rva_bytes(&mut bytes, 0x15110, b"A\0");
    bytes
}

#[test]
fn aliased_empty_lookup_tables_still_obey_descriptor_count_limit() {
    assert!(Image::parse(descriptor_count_fixture(4096)).unwrap().imports().is_empty());
    assert!(Image::parse(descriptor_count_fixture(4097)).is_err());
}

fn many_imports_fixture(count: usize, dll_len: usize) -> Vec<u8> {
    let mut bytes = imports_fixture();
    bytes.resize(0x85200, 0);
    bytes[0x200..].fill(0);
    put32(&mut bytes, SECTION + 8, 0x85000);
    put32(&mut bytes, SECTION + 16, 0x85000);
    put32(&mut bytes, OPTIONAL + 56, 0x86000);
    rva32(&mut bytes, 0x1000, 0x3000);
    rva32(&mut bytes, 0x100c, 0x1100);
    rva32(&mut bytes, 0x1010, 0x44000);
    rva_bytes(&mut bytes, 0x1100, &vec![b'A'; dll_len]);
    for index in 0..count {
        rva32(&mut bytes, 0x3000 + (index * 4) as u32, 0x8000_0001);
        rva32(&mut bytes, 0x44000 + (index * 4) as u32, 0x8000_0001);
    }
    bytes
}

#[test]
fn aliased_imports_obey_total_count_and_copied_metadata_limits() {
    assert_eq!(Image::parse(many_imports_fixture(2, 1)).unwrap().imports().len(), 2);
    let count = Image::parse(many_imports_fixture(65_537, 1)).unwrap_err();
    assert!(count.message.contains("import count exceeds"));
    let metadata = Image::parse(many_imports_fixture(2049, 4096)).unwrap_err();
    assert!(metadata.message.contains("metadata exceeds"));
}
