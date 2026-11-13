//! Owning preflight of this initial Module's normal imports, before all effects.
//!
//! This consumes the actual Module and privately retains an UNADVANCED Entry.
//! No binding, IAT store, source self-patch, driver, CPU/stack or CRT is performed.
//! There is no existing-Entry adoption, public mutable image or "imports bound"
//! constructor. [`installed`] retains actual service leases and resolves every
//! dependency/request before installing logical words in this same Entry.
//! Gdi32's sole private logical CreateCompatibleDC entry requires the explicit
//! owning raster installer profile. Existing install APIs retain it disabled;
//! resolution grants no current-screen source, HDC ABI or guest execution.
//!
//! Profile: unbound normal imports in readable/writable file-backed sections,
//! at most4096 imports and4096 descriptors. Bound/delay imports and forwarder
//! chains are not processed. The unbound profile admits forwarder fields0/FFFFFFFF.
//! The format reader supports more metadata than this executable-entry profile.
//! IAT/lookup equality is checked against retained bytes, not a snapshot or word
//! supplied by a caller. Format reference:
//! <https://learn.microsoft.com/en-us/windows/win32/debug/pe-format#the-idata-section>.

use super::{Module, ModuleOrigin, entry::Entry};
use anyhow::{Context, Result, ensure};
use sims_formats::pe32::{Image, ImportSymbol};
use std::{fmt, ops::Range};

pub mod installed;

const MAX_IMPORTS: usize = 4096;
const MAX_DESCRIPTORS: usize = 4096;
const READ_WRITE: u32 = 0xc000_0000;
const ENTRY_RANGES: [(u32, u32); 3] = [
    (0x0071_406e, 0x0071_406e + 77),
    (0x0071_400d, 0x0071_400e),
    (0x0071_412b, 0x0071_4130),
];

/// Failure retains the exact consumed read, including its origin and buffers.
#[derive(Debug)]
pub struct BeginError {
    pub error: anyhow::Error,
    pub module: Module,
}
impl fmt::Display for BeginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)
    }
}
impl std::error::Error for BeginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

#[derive(Debug)]
struct Slot {
    address: u32,
    initial_word: u32,
    descriptor: usize,
}

#[derive(Debug)]
struct DescriptorInfo {
    library_rva: u32,
    library_len: usize,
    first: usize,
    count: usize,
    lookup_rva: u32,
    iat_address: u32,
}

/// Not Clone or serializable. Every view borrows this owner; initial metadata
/// and its original read lifetime stay alive together. This type has no advance,
/// binding-return acceptance, entry extraction or driver-call accessor.
#[derive(Debug)]
pub struct ImportPlan {
    entry: Entry,
    slots: Vec<Slot>,
    descriptors: Vec<DescriptorInfo>,
    terminators: Vec<u32>,
}

impl ImportPlan {
    pub(super) fn begin(module: Module) -> std::result::Result<Self, BeginError> {
        let prepared = match preflight(&module) {
            Ok(prepared) => prepared,
            Err(error) => return Err(BeginError { error, module }),
        };
        // Entry allocates/validates but does not advance or self-patch here.
        let entry = module.begin_entry()
            .map_err(|error| BeginError { error: error.error, module: error.module })?;
        Ok(Self { entry, slots: prepared.slots, descriptors: prepared.descriptors,
            terminators: prepared.terminators })
    }

    pub fn module(&self) -> &Module { self.entry.module() }
    pub fn len(&self) -> usize { self.slots.len() }
    pub fn is_empty(&self) -> bool { self.slots.is_empty() }
    pub fn descriptor_count(&self) -> usize { self.descriptors.len() }
    /// Initial zero sentinels only; not completion or bound-import evidence.
    pub fn terminators(&self) -> &[u32] { &self.terminators }

    /// Borrow the exact request source without minting a resolved word/call.
    ///
    /// A request cannot escape its plan:
    /// ```compile_fail
    /// use sims_content::native_bootstrap::executable::read;
    /// use std::path::Path;
    /// let request = {
    ///     let plan = read(Path::new("authored.exe")).unwrap().begin_initial_imports().unwrap();
    ///     plan.request(0).unwrap()
    /// };
    /// let _ = request.slot_address();
    /// ```
    pub fn request(&self, index: usize) -> Option<Request<'_>> {
        self.slots.get(index).map(|_| Request { plan: self, index })
    }

    pub fn requests(&self) -> impl ExactSizeIterator<Item = Request<'_>> + '_ {
        (0..self.len()).map(|index| Request { plan: self, index })
    }

    /// Even a descriptor with zero symbols retains its library dependency.
    pub fn descriptors(&self) -> impl ExactSizeIterator<Item = Descriptor<'_>> + '_ {
        (0..self.descriptor_count()).map(|index| Descriptor { plan: self, index })
    }
}

pub struct Descriptor<'a> {
    plan: &'a ImportPlan,
    index: usize,
}
impl Descriptor<'_> {
    pub fn library(&self) -> &[u8] {
        let value = &self.plan.descriptors[self.index];
        self.plan.module().image().file_backed_rva(value.library_rva, value.library_len)
            .expect("validated immutable descriptor library span")
    }
    pub fn lookup_rva(&self) -> u32 { self.plan.descriptors[self.index].lookup_rva }
    pub fn iat_address(&self) -> u32 { self.plan.descriptors[self.index].iat_address }
    pub fn requests(&self) -> impl ExactSizeIterator<Item = Request<'_>> + '_ {
        let value = &self.plan.descriptors[self.index];
        (value.first..value.first + value.count).map(|index| Request { plan: self.plan, index })
    }
}

/// Immutable borrowed initial-import metadata. Copies of exposed names, origin
/// or numeric words cannot create a plan or authorize a binding/source call.
pub struct Request<'a> {
    plan: &'a ImportPlan,
    index: usize,
}
impl Request<'_> {
    pub fn slot_address(&self) -> u32 { self.plan.slots[self.index].address }
    pub fn initial_word(&self) -> u32 { self.plan.slots[self.index].initial_word }
    pub fn descriptor_index(&self) -> usize { self.plan.slots[self.index].descriptor }
    pub fn library(&self) -> &[u8] { &self.plan.module().image().imports()[self.index].dll }
    pub fn symbol(&self) -> &ImportSymbol { &self.plan.module().image().imports()[self.index].symbol }
    pub fn origin(&self) -> ModuleOrigin { self.plan.module().origin() }
}
impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitialImportRequest")
            .field("index", &self.index).field("slot", &self.slot_address())
            .field("initial_word", &self.initial_word()).finish_non_exhaustive()
    }
}

struct Prepared {
    slots: Vec<Slot>,
    descriptors: Vec<DescriptorInfo>,
    terminators: Vec<u32>,
}

fn dword(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset.checked_add(4).context("import DWORD offset overflows")?;
    let bytes: [u8; 4] = bytes.get(offset..end).context("truncated import DWORD")?
        .try_into().unwrap();
    Ok(u32::from_le_bytes(bytes))
}

fn at(image: &Image, start: u32, index: usize) -> Result<(u32, u32)> {
    let delta = u32::try_from(index.checked_mul(4).context("import offset overflows")?)?;
    let rva = start.checked_add(delta).context("import RVA overflows")?;
    let value = dword(image.file_backed_rva(rva, 4)?, 0)?;
    Ok((rva, value))
}

fn c_string(image: &Image, rva: u32, expected: &[u8]) -> Result<()> {
    let length = expected.len().checked_add(1).context("import string extent overflows")?;
    let actual = image.file_backed_rva(rva, length)?;
    ensure!(actual.last() == Some(&0) && &actual[..expected.len()] == expected,
        "import metadata string differs from the retained file");
    Ok(())
}

fn library_length(image: &Image, rva: u32) -> Result<usize> {
    let tail = image.initial_bytes().get(usize::try_from(rva)?..)
        .context("library string starts outside initial image")?;
    let length = tail.iter().take(4097).position(|byte| *byte == 0)
        .context("unterminated or over-limit library string")?;
    ensure!(length != 0, "empty import library string");
    image.file_backed_rva(rva, length + 1)?;
    Ok(length)
}

fn eligible_iat(image: &Image, rva: u32) -> Result<Range<u32>> {
    let end = rva.checked_add(4).context("IAT RVA extent overflows")?;
    let base = image.header().image_base;
    let address = base.checked_add(rva).context("IAT VA overflows")?;
    let end_address = address.checked_add(4).context("IAT VA extent overflows")?;
    image.initial_rva(rva, 4)?;
    image.file_backed_rva(rva, 4)?;
    ensure!(image.sections().iter().any(|section| {
        let bound = section.virtual_address.checked_add(
            section.virtual_size.max(section.size_of_raw_data));
        rva >= section.virtual_address && bound.is_some_and(|bound| end <= bound)
            && section.characteristics & READ_WRITE == READ_WRITE
    }), "IAT requires one readable writable declared section");
    ensure!(ENTRY_RANGES.iter().all(|(start, end)| address >= *end || end_address <= *start),
        "IAT overlaps protected entry-image source");
    if let Some(directory) = image.data_directories().get(12).filter(|d| d.size != 0) {
        let declared_end = directory.virtual_address.checked_add(directory.size)
            .context("declared IAT extent overflows")?;
        ensure!(rva >= directory.virtual_address && end <= declared_end,
            "IAT slot or terminator lies outside declared IAT directory");
    }
    Ok(address..end_address)
}

fn preflight(module: &Module) -> Result<Prepared> {
    let image = module.image();
    let imports = image.imports();
    ensure!(imports.len() <= MAX_IMPORTS, "normal import count exceeds executable profile");
    for index in [11, 13] {
        ensure!(image.data_directories().get(index).is_none_or(|d| d.size == 0),
            "bound or delay import directories require another loader profile");
    }
    let mut slots = Vec::new();
    slots.try_reserve_exact(imports.len()).context("reserve import slot history")?;
    let mut terminators = Vec::new();
    let mut descriptors = Vec::new();
    let mut ranges = Vec::new();
    let Some(directory) = image.data_directories().get(1).filter(|d| d.size != 0) else {
        ensure!(imports.is_empty(), "import metadata without a directory");
        ensure!(image.data_directories().get(12).is_none_or(|d| d.size == 0),
            "IAT directory has no normal import descriptors");
        return Ok(Prepared { slots, descriptors, terminators });
    };
    let table = image.file_backed_rva(directory.virtual_address, usize::try_from(directory.size)?)?;
    let mut terminated = false;
    for descriptor in table.chunks_exact(20) {
        if descriptor.iter().all(|b| *b == 0) { terminated = true; break; }
        ensure!(descriptors.len() < MAX_DESCRIPTORS, "import descriptor count exceeds executable profile");
        let lookup = dword(descriptor, 0)?;
        ensure!(dword(descriptor, 4)? == 0, "bound import timestamp is not initial lookup state");
        ensure!(matches!(dword(descriptor, 8)?, 0 | u32::MAX),
            "forwarder chains require another loader profile");
        let library = dword(descriptor, 12)?;
        let library_len = library_length(image, library)?;
        let iat = dword(descriptor, 16)?;
        let lookup = if lookup == 0 { iat } else { lookup };
        let first = slots.len();
        let mut index = 0;
        loop {
            ensure!(slots.len() <= imports.len(), "import slot traversal exceeds parsed metadata");
            let (_, original) = at(image, lookup, index)?;
            let (rva, preimage) = at(image, iat, index)?;
            let range = eligible_iat(image, rva)?;
            ensure!(preimage == original, "initial IAT differs from its unbound lookup word");
            ranges.try_reserve(1).context("reserve IAT alias check")?;
            ranges.push(range.clone());
            if original == 0 {
                terminators.try_reserve(1).context("reserve IAT terminator history")?;
                terminators.push(range.start);
                break;
            }
            let metadata = imports.get(slots.len()).context("import traversal has no parsed request")?;
            ensure!(metadata.iat_va == range.start, "import slot metadata disagrees with descriptor order");
            c_string(image, library, &metadata.dll)?;
            match &metadata.symbol {
                ImportSymbol::Ordinal(ordinal) => ensure!(original == (0x8000_0000 | u32::from(*ordinal)),
                    "ordinal request differs from initial lookup word"),
                ImportSymbol::Name { hint, name } => {
                    ensure!(original & 0x8000_0000 == 0, "named request has an ordinal lookup word");
                    ensure!(image.file_backed_rva(original, 2)? == hint.to_le_bytes(),
                        "import name hint differs from initial file");
                    c_string(image, original.checked_add(2).context("hint/name RVA overflows")?, name)?;
                }
            }
            slots.push(Slot { address: range.start, initial_word: preimage, descriptor: descriptors.len() });
            index = index.checked_add(1).context("import slot index overflows")?;
        }
        descriptors.try_reserve(1).context("reserve import descriptor history")?;
        descriptors.push(DescriptorInfo { library_rva: library, library_len,
            first, count: slots.len() - first, lookup_rva: lookup,
            iat_address: image.header().image_base.checked_add(iat).context("IAT VA overflows")? });
    }
    ensure!(terminated && slots.len() == imports.len(), "incomplete import descriptor traversal");
    ranges.sort_unstable_by_key(|range| range.start);
    ensure!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start),
        "overlapping or aliased IAT slots/terminators");
    Ok(Prepared { slots, descriptors, terminators })
}

#[cfg(test)]
#[path = "imports/tests.rs"]
mod tests;

