//! An owning, bounded PE32/I386 reader for **initial-file logical bytes**.
//!
//! The mapping is immutable and uses the preferred image base. It copies all
//! declared raw section bytes (including padding) and leaves BSS and reserved
//! gaps zeroed. A zero here is not evidence of a constructor-entry value. This
//! is neither a Windows loader nor executable identity: imports are metadata;
//! relocation, resolution, TLS, CRT initialization and execution never occur.
//!
//! This strict profile requires normal page alignment, a bounded optional-header
//! directory table, and nonempty declared section spans. Queries exclude gaps
//! and alignment tails even though `initial_bytes` retains the whole image.
//! Format reference: <https://learn.microsoft.com/en-us/windows/win32/debug/pe-format>.

use crate::{Error, Result};
use std::fmt;

pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

// These additional profile bounds also limit work for repeated/aliased import
// tables and strings. All derived allocations are fallible.
const MAX_IMPORT_DESCRIPTORS: usize = 4096;
const MAX_IMPORTS: usize = 65_536;
const MAX_IMPORT_NAME_BYTES: usize = 4096;
const MAX_IMPORT_METADATA_BYTES: usize = 8 * 1024 * 1024;

/// Read-only metadata describing the preferred-base initial logical image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub image_base: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub address_of_entry_point: u32,
    pub section_alignment: u32,
    pub file_alignment: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub size_of_raw_data: u32,
    pub pointer_to_raw_data: u32,
    pub characteristics: u32,
}

/// Index 4 uses a file offset, not an RVA. Contents are not initialized or run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirectory {
    pub virtual_address: u32,
    pub size: u32,
}

/// Raw DLL and symbol bytes have no encoding or filesystem authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub dll: Vec<u8>,
    /// Preferred VA of the initial IAT slot, never a resolved/callable address.
    pub iat_va: u32,
    pub symbol: ImportSymbol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSymbol {
    Name { hint: u16, name: Vec<u8> },
    Ordinal(u16),
}

/// Owns exactly the consumed file bytes and their immutable logical mapping.
/// There is no mutable accessor, bytes adoption after parsing, or `Clone`.
pub struct Image {
    raw: Vec<u8>,
    initial: Vec<u8>,
    header: Header,
    sections: Vec<Section>,
    directories: Vec<DataDirectory>,
    imports: Vec<Import>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("raw_len", &self.raw.len())
            .field("initial_len", &self.initial.len())
            .field("header", &self.header)
            .field("section_count", &self.sections.len())
            .field("directory_count", &self.directories.len())
            .field("import_count", &self.imports.len())
            .finish()
    }
}

impl Image {
    /// Consume one file buffer, validate the strict profile, then map its
    /// initial bytes. Any failure returns no partially parsed `Image`.
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::new(0, "PE file exceeds 64 MiB limit"));
        }
        if take(&bytes, 0, 2)? != b"MZ" {
            return Err(Error::new(0, "missing MZ signature"));
        }
        take(&bytes, 0, 64)?;
        let pe = size(u32_at(&bytes, 0x3c)?)?;
        if pe < 64 {
            return Err(Error::new(0x3c, "PE header overlaps DOS header"));
        }
        if take(&bytes, pe, 4)? != b"PE\0\0" {
            return Err(Error::new(pe, "missing PE signature"));
        }
        let coff_offset = add(pe, 4)?;
        let coff = take(&bytes, coff_offset, 20)?;
        if u16_at(coff, 0)? != 0x014c {
            return Err(Error::new(coff_offset, "only I386 images are supported"));
        }
        let section_count = usize::from(u16_at(coff, 2)?);
        if !(1..=96).contains(&section_count) {
            return Err(Error::new(coff_offset, "section count must be 1..=96"));
        }
        let characteristics = u16_at(coff, 18)?;
        if characteristics & 0x0002 == 0 || characteristics & 0x8080 != 0 {
            return Err(Error::new(coff_offset, "unsupported image characteristics"));
        }
        let optional_size = usize::from(u16_at(coff, 16)?);
        let optional_offset = add(coff_offset, 20)?;
        let optional = take(&bytes, optional_offset, optional_size)?;
        if optional_size < 96 || u16_at(optional, 0)? != 0x010b {
            return Err(Error::new(optional_offset, "only complete PE32 headers are supported"));
        }
        let directory_count = size(u32_at(optional, 92)?)?;
        if directory_count > 16
            || add(96, multiply(directory_count, 8)?)? > optional_size
        {
            return Err(Error::new(optional_offset, "inconsistent optional-header directory table"));
        }
        let header = Header {
            image_base: u32_at(optional, 28)?,
            size_of_image: u32_at(optional, 56)?,
            size_of_headers: u32_at(optional, 60)?,
            address_of_entry_point: u32_at(optional, 16)?,
            section_alignment: u32_at(optional, 32)?,
            file_alignment: u32_at(optional, 36)?,
        };
        if !header.section_alignment.is_power_of_two()
            || header.section_alignment < 4096
            || !header.file_alignment.is_power_of_two()
            || !(512..=65_536).contains(&header.file_alignment)
            || header.section_alignment < header.file_alignment
            || header.image_base % 65_536 != 0
        {
            return Err(Error::new(optional_offset, "unsupported PE alignment profile"));
        }
        let image_size = size(header.size_of_image)?;
        let headers_size = size(header.size_of_headers)?;
        if image_size == 0
            || image_size > MAX_IMAGE_BYTES
            || header.size_of_image % header.section_alignment != 0
            || headers_size == 0
            || header.size_of_headers % header.file_alignment != 0
            || headers_size > image_size
            || headers_size > bytes.len()
        {
            return Err(Error::new(optional_offset, "invalid image/header extent"));
        }
        header.image_base.checked_add(header.size_of_image)
            .ok_or_else(|| Error::new(optional_offset, "preferred VA extent wraps"))?;
        let section_table = add(optional_offset, optional_size)?;
        let table_end = add(section_table, multiply(section_count, 40)?)?;
        if table_end > headers_size {
            return Err(Error::new(section_table, "headers do not cover all tables"));
        }
        take(&bytes, section_table, multiply(section_count, 40)?)?;

        let mut sections: Vec<Section> = Vec::new();
        reserve(&mut sections, section_count, section_table)?;
        let mut reserved_end = align_up(header.size_of_headers, header.section_alignment)?;
        for index in 0..section_count {
            let offset = add(section_table, multiply(index, 40)?)?;
            let record = take(&bytes, offset, 40)?;
            let mut name = [0; 8];
            name.copy_from_slice(take(record, 0, 8)?);
            let section = Section {
                name,
                virtual_size: u32_at(record, 8)?,
                virtual_address: u32_at(record, 12)?,
                size_of_raw_data: u32_at(record, 16)?,
                pointer_to_raw_data: u32_at(record, 20)?,
                characteristics: u32_at(record, 36)?,
            };
            let span = section.virtual_size.max(section.size_of_raw_data);
            if span == 0
                || section.virtual_address % header.section_alignment != 0
                || section.virtual_address < reserved_end
            {
                return Err(Error::new(offset, "empty, overlapping or unaligned section"));
            }
            let end = section.virtual_address.checked_add(span)
                .ok_or_else(|| Error::new(offset, "section RVA extent wraps"))?;
            reserved_end = align_up(end, header.section_alignment)?;
            if reserved_end > header.size_of_image {
                return Err(Error::new(offset, "section exceeds SizeOfImage"));
            }
            if section.size_of_raw_data == 0 {
                if section.pointer_to_raw_data != 0 {
                    return Err(Error::new(offset, "empty raw span has a file pointer"));
                }
            } else {
                if section.size_of_raw_data % header.file_alignment != 0
                    || section.pointer_to_raw_data % header.file_alignment != 0
                    || section.pointer_to_raw_data < header.size_of_headers
                {
                    return Err(Error::new(offset, "invalid section raw alignment/offset"));
                }
                let raw_end = section.pointer_to_raw_data.checked_add(section.size_of_raw_data)
                    .ok_or_else(|| Error::new(offset, "section file extent wraps"))?;
                take(&bytes, size(section.pointer_to_raw_data)?, size(section.size_of_raw_data)?)?;
                for previous in &sections {
                    let previous_end = previous.pointer_to_raw_data
                        .checked_add(previous.size_of_raw_data)
                        .ok_or_else(|| Error::new(offset, "section file extent wraps"))?;
                    if previous.size_of_raw_data != 0
                        && section.pointer_to_raw_data < previous_end
                        && previous.pointer_to_raw_data < raw_end
                    {
                        return Err(Error::new(offset, "overlapping section file spans"));
                    }
                }
            }
            sections.push(section);
        }

        let mut directories = Vec::new();
        reserve(&mut directories, directory_count, optional_offset)?;
        for index in 0..directory_count {
            let offset = add(96, multiply(index, 8)?)?;
            directories.push(DataDirectory {
                virtual_address: u32_at(optional, offset)?,
                size: u32_at(optional, add(offset, 4)?)?,
            });
        }

        let mut initial = Vec::new();
        reserve(&mut initial, image_size, optional_offset)?;
        initial.resize(image_size, 0);
        initial.get_mut(..headers_size)
            .ok_or_else(|| Error::new(0, "invalid mapped header span"))?
            .copy_from_slice(take(&bytes, 0, headers_size)?);
        for section in &sections {
            let len = size(section.size_of_raw_data)?;
            if len == 0 {
                continue;
            }
            let start = size(section.virtual_address)?;
            let end = add(start, len)?;
            initial.get_mut(start..end)
                .ok_or_else(|| Error::new(start, "invalid mapped raw span"))?
                .copy_from_slice(take(&bytes, size(section.pointer_to_raw_data)?, len)?);
        }
        let mut image = Self {
            raw: bytes,
            initial,
            header,
            sections,
            directories,
            imports: Vec::new(),
        };
        for (index, directory) in image.directories.iter().enumerate() {
            if (directory.virtual_address == 0) != (directory.size == 0) {
                return Err(Error::new(optional_offset, "incoherent zero directory pair"));
            }
            if directory.size == 0 {
                continue;
            }
            if index == 4 {
                // Certificate bytes stay in raw(); this field is a file offset.
                if directory.virtual_address % 8 != 0 {
                    return Err(Error::new(optional_offset, "unaligned certificate file offset"));
                }
                directory.virtual_address.checked_add(directory.size)
                    .ok_or_else(|| Error::new(optional_offset, "certificate file span wraps"))?;
                take(&image.raw, size(directory.virtual_address)?, size(directory.size)?)?;
            } else {
                image.initial_rva(directory.virtual_address, size(directory.size)?)?;
            }
        }
        if image.header.address_of_entry_point != 0 {
            image.initial_rva(image.header.address_of_entry_point, 1)?;
        }
        image.imports = parse_imports(&image)?;
        Ok(image)
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Whole logical buffer, including zero reserved gaps that queries exclude.
    pub fn initial_bytes(&self) -> &[u8] {
        &self.initial
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn data_directories(&self) -> &[DataDirectory] {
        &self.directories
    }

    pub fn imports(&self) -> &[Import] {
        &self.imports
    }

    /// A nonempty span wholly inside headers OR one declared section span.
    /// Raw padding and BSS are included; reserved tails and crossings are not.
    pub fn initial_rva(&self, rva: u32, len: usize) -> Result<&[u8]> {
        let end = rva_end(rva, len)?;
        let in_headers = rva < self.header.size_of_headers && end <= self.header.size_of_headers;
        let in_section = self.sections.iter().any(|section| {
            let section_end = section.virtual_address
                .checked_add(section.virtual_size.max(section.size_of_raw_data));
            rva >= section.virtual_address && section_end.is_some_and(|bound| end <= bound)
        });
        if !in_headers && !in_section {
            return Err(Error::new(size(rva)?, "RVA range is not one declared span"));
        }
        take(&self.initial, size(rva)?, len)
    }

    /// Preferred-base VA query at the initial-file logical stage only.
    pub fn initial_va(&self, va: u32, len: usize) -> Result<&[u8]> {
        let rva = va.checked_sub(self.header.image_base)
            .ok_or_else(|| Error::new(0, "VA is below preferred image base"))?;
        // Explicitly check the requested preferred VA extent as well as its RVA.
        rva_end(va, len)?;
        self.initial_rva(rva, len)
    }

    /// Exact original bytes; never supplies an implicit BSS/alignment zero.
    pub fn file_backed_rva(&self, rva: u32, len: usize) -> Result<&[u8]> {
        rva_end(rva, len)?;
        take(self.file_window(rva)?, 0, len)
    }

    // Remaining file bytes in this one header/raw section span. Import parsing
    // uses the same boundary for both payloads and their required terminators.
    fn file_window(&self, rva: u32) -> Result<&[u8]> {
        if rva < self.header.size_of_headers {
            return take(&self.raw, size(rva)?, size(self.header.size_of_headers - rva)?);
        }
        for section in &self.sections {
            if let Some(delta) = rva.checked_sub(section.virtual_address) {
                if delta < section.size_of_raw_data {
                    let start = section.pointer_to_raw_data.checked_add(delta)
                        .ok_or_else(|| Error::new(0, "file offset wraps"))?;
                    return take(&self.raw, size(start)?, size(section.size_of_raw_data - delta)?);
                }
            }
        }
        Err(Error::new(size(rva)?, "RVA is not file-backed"))
    }
}

fn parse_imports(image: &Image) -> Result<Vec<Import>> {
    let mut imports = Vec::new();
    let mut metadata_bytes = 0;
    let directory = match image.directories.get(1) {
        Some(directory) if directory.size != 0 => directory,
        _ => return Ok(imports),
    };
    let table = image.file_backed_rva(directory.virtual_address, size(directory.size)?)?;
    for (index, descriptor) in table.chunks_exact(20).enumerate() {
        if descriptor.iter().all(|&byte| byte == 0) {
            return Ok(imports);
        }
        if index >= MAX_IMPORT_DESCRIPTORS {
            return Err(Error::new(0, "import descriptor count exceeds profile limit"));
        }
        let original_first_thunk = u32_at(descriptor, 0)?;
        let timestamp = u32_at(descriptor, 4)?;
        let name_rva = u32_at(descriptor, 12)?;
        let first_thunk = u32_at(descriptor, 16)?;
        if name_rva == 0 || first_thunk == 0 {
            return Err(Error::new(0, "missing import name/IAT"));
        }
        let lookup_rva = if original_first_thunk == 0 {
            if timestamp != 0 {
                return Err(Error::new(0, "bound IAT without OriginalFirstThunk is unsupported"));
            }
            first_thunk
        } else {
            original_first_thunk
        };
        // Decode DWORDs from bounded byte slices; their starting RVAs need
        // not be aligned host pointers. This is metadata, never callable IATs.
        let dll = c_string(image.file_window(name_rva)?)?;
        let lookup = image.file_window(lookup_rva)?;
        let iat = image.file_window(first_thunk)?;
        let slots = (lookup.len() / 4).min(iat.len() / 4);
        let mut terminated = false;
        for index in 0..slots {
            let offset = multiply(index, 4)?;
            let thunk = u32_at(lookup, offset)?;
            let iat_value = u32_at(iat, offset)?;
            let slot_rva = first_thunk.checked_add(u32::try_from(offset)
                .map_err(|_| Error::new(offset, "IAT slot offset exceeds u32"))?)
                .ok_or_else(|| Error::new(offset, "IAT slot RVA wraps"))?;
            let iat_va = image.header.image_base.checked_add(slot_rva)
                .ok_or_else(|| Error::new(offset, "IAT slot VA wraps"))?;
            rva_end(iat_va, 4)?;
            if thunk == 0 {
                if iat_value != 0 {
                    return Err(Error::new(offset, "IAT terminator disagrees with lookup table"));
                }
                terminated = true;
                break;
            }
            if imports.len() >= MAX_IMPORTS {
                return Err(Error::new(offset, "import count exceeds profile limit"));
            }
            if iat_value == 0 {
                return Err(Error::new(offset, "IAT ends before lookup table"));
            }
            let symbol = if thunk & 0x8000_0000 != 0 {
                if thunk & 0x7fff_0000 != 0 {
                    return Err(Error::new(offset, "reserved ordinal thunk bits are set"));
                }
                ImportSymbol::Ordinal((thunk & 0xffff) as u16)
            } else {
                let hint_name = image.file_window(thunk)?;
                let hint = u16_at(hint_name, 0)?;
                let name = c_string(hint_name.get(2..)
                    .ok_or_else(|| Error::new(offset, "truncated hint/name"))?)?;
                ImportSymbol::Name {
                    hint,
                    name: copy_metadata(name, &mut metadata_bytes)?,
                }
            };
            let dll = copy_metadata(dll, &mut metadata_bytes)?;
            imports.try_reserve(1)
                .map_err(|_| Error::new(offset, "PE import allocation failed"))?;
            imports.push(Import { dll, iat_va, symbol });
        }
        if !terminated {
            return Err(Error::new(0, "unterminated file-backed import lookup/IAT"));
        }
    }
    Err(Error::new(0, "import descriptor table has no in-span terminator"))
}

fn c_string(window: &[u8]) -> Result<&[u8]> {
    let end = window.iter().take(MAX_IMPORT_NAME_BYTES + 1)
        .position(|&byte| byte == 0)
        .ok_or_else(|| Error::new(0, "unterminated or over-limit import name"))?;
    if end == 0 {
        return Err(Error::new(0, "empty import name"));
    }
    take(window, 0, end)
}

fn copy_metadata(bytes: &[u8], used: &mut usize) -> Result<Vec<u8>> {
    let total = add(*used, bytes.len())?;
    if total > MAX_IMPORT_METADATA_BYTES {
        return Err(Error::new(0, "import string metadata exceeds 8 MiB limit"));
    }
    let mut result = Vec::new();
    reserve(&mut result, bytes.len(), 0)?;
    result.extend_from_slice(bytes);
    *used = total;
    Ok(result)
}

fn reserve<T>(target: &mut Vec<T>, additional: usize, offset: usize) -> Result<()> {
    target.try_reserve_exact(additional)
        .map_err(|_| Error::new(offset, "PE derived allocation failed"))
}

fn take(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = add(offset, len)?;
    bytes.get(offset..end).ok_or_else(|| Error::new(offset, "truncated PE span"))
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = take(bytes, offset, 2)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = take(bytes, offset, 4)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn size(value: u32) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::new(0, "PE size exceeds usize"))
}

fn add(left: usize, right: usize) -> Result<usize> {
    left.checked_add(right).ok_or_else(|| Error::new(left, "PE offset overflow"))
}

fn multiply(left: usize, right: usize) -> Result<usize> {
    left.checked_mul(right).ok_or_else(|| Error::new(left, "PE count overflow"))
}

fn rva_end(rva: u32, len: usize) -> Result<u32> {
    if len == 0 {
        return Err(Error::new(size(rva)?, "zero-length PE queries are unsupported"));
    }
    let length = u32::try_from(len).map_err(|_| Error::new(0, "PE range length exceeds u32"))?;
    rva.checked_add(length).ok_or_else(|| Error::new(0, "PE address range wraps"))
}

fn align_up(value: u32, alignment: u32) -> Result<u32> {
    // Callers have already validated this nonzero power-of-two alignment.
    value.checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
        .ok_or_else(|| Error::new(0, "PE aligned extent wraps"))
}

#[cfg(test)]
mod tests;
