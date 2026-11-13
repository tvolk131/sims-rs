//! Owning, preferred-base **module-image** prefix of the initial EXE entry.
//!
//! This records only the two verified self-patch stores, then stops BEFORE
//! `7140b6 -> 713ce0`. It does not execute the prologue, register saves, stack
//! pushes, protection driver, imports, protected code, CRT or constructors.
//! Call arguments are source observations, not a materialized caller/stack.
//! Neither this prefix nor its cut qualifies counters or a completed Application.

use super::{Module, ModuleOrigin};
use anyhow::{Context, Result, ensure};
use std::{fmt, ops::Range, sync::Arc};

const BASE: u32 = 0x0040_0000;
const ENTRY: u32 = 0x0071_406e;
const FLAG: u32 = 0x0071_400d;
const JUMP: u32 = 0x0071_412b;
const PATCHED_TARGET: u32 = 0x0063_8364;
const CALL: u32 = 0x0071_40b6;
const DRIVER: u32 = 0x0071_3ce0;
const CONFIG: u32 = 0x0071_3fd9;
const READ: u32 = 0x4000_0000;
const EXECUTE: u32 = 0x2000_0000;
const WRITE: u32 = 0x8000_0000;
const STXT_CHARACTERISTICS: u32 = 0xe000_0020;

const INITIAL_SOURCE: [u8; 77] = [
    0x55, 0x8b, 0xec, 0x60, 0xbb, 0x6e, 0x40, 0x71, 0x00, 0xb8, 0x0d, 0x40, 0x71,
    0x00, 0x33, 0xc9, 0x8a, 0x08, 0x85, 0xc9, 0x74, 0x0c, 0xb8, 0xe4, 0x40, 0x71,
    0x00, 0x2b, 0xc3, 0x83, 0xe8, 0x05, 0xeb, 0x0e, 0x51, 0xb9, 0x2b, 0x41, 0x71,
    0x00, 0x8b, 0xc1, 0x2b, 0xc3, 0x03, 0x41, 0x01, 0x59, 0xc6, 0x03, 0xe9, 0x89,
    0x43, 0x01, 0x51, 0x68, 0xd9, 0x3f, 0x71, 0x00, 0x33, 0xc0, 0x85, 0xc9, 0x74,
    0x05, 0x8b, 0x45, 0x08, 0xeb, 0x00, 0x50, 0xe8, 0x25, 0xfc, 0xff, 0xff,
];
const INITIAL_JUMP: [u8; 5] = [0xe9, 0x34, 0x42, 0xf2, 0xff];

/// The sole supported boundary. Reaching it does not execute its pending call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    BeforeProtectionDriver,
}

/// A refused begin returns the exact consumed owner, with its bytes, descriptor
/// observation and origin intact. It never substitutes a newly read Module.
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

/// One exact module-image store and its actual preimage. History is immutable.
#[derive(Debug)]
pub struct Write {
    instruction: u32,
    address: u32,
    before: [u8; 4],
    after: [u8; 4],
    len: usize,
}

impl Write {
    pub fn instruction(&self) -> u32 {
        self.instruction
    }

    pub fn address(&self) -> u32 {
        self.address
    }

    pub fn before(&self) -> &[u8] {
        &self.before[..self.len]
    }

    pub fn after(&self) -> &[u8] {
        &self.after[..self.len]
    }
}

// A fresh allocation for each Entry, retained even by historical observations.
// It cannot be rebound to another Entry; moving the owning Entry is harmless.
struct CallIdentity {
    origin: ModuleOrigin,
}

/// Opaque observation of this Entry's one pending source call. Cloning retains
/// pointer identity, never module access or permission to execute/initialize it.
/// There is no public constructor or serialized form.
#[derive(Clone)]
pub struct ProtectionCall {
    identity: Arc<CallIdentity>,
}

impl ProtectionCall {
    pub fn instruction(&self) -> u32 {
        CALL
    }

    pub fn target(&self) -> u32 {
        DRIVER
    }

    /// Source arguments at the cut, without caller-frame or stack authority.
    /// The second address is inside the retained image, not resolved content.
    pub fn arguments(&self) -> [u32; 2] {
        [0, CONFIG]
    }

    pub fn origin(&self) -> ModuleOrigin {
        self.identity.origin.clone()
    }

    pub fn same_call(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

impl fmt::Debug for ProtectionCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtectionCall")
            .field("instruction", &self.instruction())
            .field("target", &self.target())
            .field("arguments", &self.arguments())
            .field("origin", &self.origin())
            .finish_non_exhaustive()
    }
}

struct Layout {
    source: Range<usize>,
    jump: Range<usize>,
    flag: Range<usize>,
    patch: Range<usize>,
}

/// Non-Clone owner of the actual initial Module and one private mutable logical
/// image. No byte/token adoption, reset, driver return or later frontier exists.
pub struct Entry {
    module: Module,
    image: Vec<u8>,
    layout: Layout,
    writes: Vec<Write>,
    call: ProtectionCall,
    at_cut: bool,
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("module", &self.module)
            .field("writes", &self.writes)
            .field("at_cut", &self.at_cut)
            .finish_non_exhaustive()
    }
}

impl Entry {
    pub(super) fn begin(module: Module) -> std::result::Result<Self, BeginError> {
        // All validation and allocations precede any image effect. Returning
        // an error moves the same Module back, including its original buffers.
        let prepared = (|| -> Result<_> {
            let layout = validate_initial(&module)?;
            let initial = module.image().initial_bytes();
            let mut image = Vec::new();
            image
                .try_reserve_exact(initial.len())
                .context("reserve bounded entry module image")?;
            image.extend_from_slice(initial);
            let mut writes = Vec::<Write>::new();
            writes
                .try_reserve_exact(2)
                .context("reserve two entry image-write records")?;
            // Call data and the fixed-size preimage records are also bounded.
            // This allocation is private until the cut and occurs before stores.
            let call = ProtectionCall {
                identity: Arc::new(CallIdentity {
                    origin: module.origin(),
                }),
            };
            Ok((layout, image, writes, call))
        })();
        match prepared {
            Ok((layout, image, writes, call)) => Ok(Self {
                module,
                image,
                layout,
                writes,
                call,
                at_cut: false,
            }),
            Err(error) => Err(BeginError { error, module }),
        }
    }

    /// Replay the two module-image stores once, stopping before the driver call.
    /// Repeated advances return this same cut/call without decoding the newly
    /// patched entry jump or replaying stores. No CPU or stack is emulated.
    pub fn advance(&mut self) -> Result<Cut> {
        if self.at_cut {
            return Ok(Cut::BeforeProtectionDriver);
        }
        ensure!(
            self.writes.is_empty() && self.writes.capacity() >= 2,
            "entry image history is not at its initial boundary"
        );
        ensure!(
            self.image.get(self.layout.source.clone()) == Some(INITIAL_SOURCE.as_slice()),
            "entry source no longer matches the supported initial instructions"
        );
        ensure!(
            self.image.get(self.layout.flag.clone()) == Some([0].as_slice()),
            "nonzero entry flag requires an unqualified incoming caller argument"
        );
        let jump = self.image
            .get(self.layout.jump.clone())
            .context("entry jump outside retained image")?;
        ensure!(jump == INITIAL_JUMP, "unsupported initial forwarding jump");
        let operand = u32::from_le_bytes(jump[1..5].try_into()?);
        // The zero-flag path uses JUMP - ENTRY + DWORD[JUMP + 1]. The -5
        // instruction belongs to the unsupported nonzero-flag path.
        let displacement = JUMP.wrapping_sub(ENTRY).wrapping_add(operand);
        ensure!(
            ENTRY.wrapping_add(5).wrapping_add(displacement) == PATCHED_TARGET,
            "unsupported entry self-patch destination"
        );
        let patch = self.image
            .get_mut(self.layout.patch.clone())
            .context("entry patch outside retained image")?;
        let first = Write {
            instruction: 0x0071_409e,
            address: ENTRY,
            before: [patch[0], 0, 0, 0],
            after: [0xe9, 0, 0, 0],
            len: 1,
        };
        let second = Write {
            instruction: 0x0071_40a1,
            address: ENTRY + 1,
            before: patch[1..5].try_into()?,
            after: displacement.to_le_bytes(),
            len: 4,
        };
        // No fallible operation or allocation remains after the first store.
        patch[0] = first.after[0];
        patch[1..5].copy_from_slice(&second.after);
        self.writes.push(first);
        self.writes.push(second);
        self.at_cut = true;
        Ok(Cut::BeforeProtectionDriver)
    }

    pub fn module(&self) -> &Module {
        &self.module
    }

    pub(super) fn current_image_bytes(&self) -> &[u8] {
        &self.image
    }

    // Only the owning import installer can construct this sealed batch. It
    // contains genuine service-issued words, checked ranges and pre-reserved
    // history; no public word setter or mutable-image borrow is introduced.
    pub(super) fn install_program_imports(
        &mut self,
        ready: &super::imports::installed::ReadyImports,
    ) -> Result<()> {
        ensure!(!self.at_cut && self.writes.is_empty(), "imports require an unadvanced Entry");
        ensure!(ready.origin() == self.module.origin(), "import batch belongs to another load");
        ensure!(self.image.as_slice() == self.module.image().initial_bytes(),
            "import installation requires the exact current initial image");
        for write in ready.writes() {
            let range = mapped_range(&self.module, write.address(), 4)?;
            ensure!(range == write.range(), "import slot extent changed");
            ensure!(self.image.get(range) == Some(write.before().as_slice()),
                "current import preimage changed");
            ensure!(write.after() != 0, "resolved import cannot be NULL");
        }
        for (address, expected_range) in ready.terminators() {
            let range = mapped_range(&self.module, *address, 4)?;
            ensure!(&range == expected_range && self.image.get(range) == Some([0; 4].as_slice()),
                "current import sentinel or extent changed");
        }
        // Every range/preimage and sentinel has been checked before the first
        // store. Byte copies support unaligned DWORDs. No allocation, service
        // operation, fallible result or history push follows the first store.
        for write in ready.writes() {
            self.image[write.range()].copy_from_slice(&write.after().to_le_bytes());
        }
        Ok(())
    }

    pub fn current_span(&self, va: u32, len: usize) -> Result<CurrentSpan<'_>> {
        let range = mapped_range(&self.module, va, len)?;
        let bytes = self.image.get(range).context("current span outside entry image")?;
        Ok(CurrentSpan {
            entry: self,
            address: va,
            bytes,
        })
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn pending_call(&self) -> Option<&ProtectionCall> {
        self.at_cut.then_some(&self.call)
    }

    /// Accept only the live owning Entry's issued call (or its observation clone).
    /// A matching address, module token or another load's bytes is insufficient.
    pub fn accepts_call(&self, call: &ProtectionCall) -> bool {
        self.at_cut && self.call.same_call(call) && call.origin() == self.module.origin()
    }
}

/// Immutable current bytes borrowed from the live Entry, distinct from the
/// Module's immutable InitialSpan. An origin token cannot construct this view.
/// Its borrow prevents mutation while the bytes remain in use:
/// ```compile_fail
/// use sims_content::native_bootstrap::executable::entry::Entry;
/// fn hold_current_bytes(entry: &mut Entry) {
///     let span = entry.current_span(0x0071_406e, 5).unwrap();
///     entry.advance().unwrap();
///     let _ = span.bytes();
/// }
/// ```
pub struct CurrentSpan<'a> {
    entry: &'a Entry,
    address: u32,
    bytes: &'a [u8],
}

impl CurrentSpan<'_> {
    pub fn address(&self) -> u32 {
        self.address
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub fn origin(&self) -> ModuleOrigin {
        self.entry.module.origin()
    }
}

fn mapped_range(module: &Module, va: u32, len: usize) -> Result<Range<usize>> {
    // Reuse the actual parsed mapping's empty/overflow/gap/tail/crossing policy.
    module.image().initial_va(va, len)?;
    let rva = va
        .checked_sub(module.image().header().image_base)
        .context("entry span below preferred base")?;
    let start = usize::try_from(rva)?;
    let end = start.checked_add(len).context("entry image offset overflow")?;
    ensure!(end <= module.image().initial_bytes().len(), "entry span outside image");
    Ok(start..end)
}

fn accessible_range(module: &Module, va: u32, len: usize, access: u32) -> Result<Range<usize>> {
    let range = mapped_range(module, va, len)?;
    let rva = u32::try_from(range.start)?;
    let end = u32::try_from(range.end)?;
    let covered = module.image().sections().iter().any(|section| {
        section.virtual_address
            .checked_add(section.virtual_size.max(section.size_of_raw_data))
            .is_some_and(|bound| rva >= section.virtual_address && end <= bound)
            && section.characteristics & access == access
    });
    ensure!(
        covered,
        "entry span {va:#x}+{len:#x} lacks required section access {access:#x}"
    );
    Ok(range)
}

fn validate_initial(module: &Module) -> Result<Layout> {
    let image = module.image();
    let header = image.header();
    ensure!(header.image_base == BASE, "unsupported entry preferred base");
    ensure!(
        header.address_of_entry_point == ENTRY - BASE,
        "unsupported initial entry RVA"
    );
    // Image's strict parser already proves PE32/I386 and executable-image bits,
    // but intentionally does not exclude DLLs. Read this bit from the same raw
    // file bytes; never reopen its observed pathname or adopt a header snapshot.
    let raw = image.raw();
    let pe = u32::from_le_bytes(
        raw.get(0x3c..0x40).context("missing owned PE offset")?.try_into()?,
    );
    let coff_flags = usize::try_from(pe)?
        .checked_add(22)
        .context("owned COFF characteristics offset overflow")?;
    let flags_end = coff_flags.checked_add(2).context("owned COFF flags extent overflow")?;
    let flags = u16::from_le_bytes(
        raw.get(coff_flags..flags_end)
            .context("missing owned COFF characteristics")?.try_into()?,
    );
    ensure!(flags & 0x2000 == 0, "initial entry requires EXE, not DLL");
    for index in [5, 9] {
        ensure!(
            image.data_directories().get(index)
                .is_none_or(|directory| directory.virtual_address == 0 && directory.size == 0),
            "entry profile requires absent relocation/TLS directories"
        );
    }
    let mut sections = image.sections().iter().filter(|section| section.name == *b"stxt371\0");
    let stxt = sections.next().context("entry profile requires stxt371")?;
    ensure!(
        sections.next().is_none() && stxt.characteristics == STXT_CHARACTERISTICS,
        "unsupported stxt371 identity or characteristics"
    );
    let source = accessible_range(module, ENTRY, INITIAL_SOURCE.len(), READ | EXECUTE)?;
    let jump = accessible_range(module, JUMP, INITIAL_JUMP.len(), READ | EXECUTE)?;
    let flag = accessible_range(module, FLAG, 1, READ)?;
    let patch = accessible_range(module, ENTRY, 5, READ | WRITE | EXECUTE)?;
    let stxt_end = stxt.virtual_address
        .checked_add(stxt.virtual_size.max(stxt.size_of_raw_data))
        .context("stxt371 extent overflow")?;
    for range in [&source, &jump, &flag, &patch] {
        ensure!(
            u32::try_from(range.start)? >= stxt.virtual_address
                && u32::try_from(range.end)? <= stxt_end,
            "entry source/flag/jump/patch must be inside stxt371"
        );
    }
    // Mapping/access checks only: do not inspect or materialize driver, config
    // contents or protected jump-destination code, and never follow either call.
    accessible_range(module, DRIVER, 1, READ | EXECUTE)?;
    accessible_range(module, CONFIG, 1, READ)?;
    accessible_range(module, PATCHED_TARGET, 1, READ | EXECUTE)?;
    ensure!(
        image.initial_va(FLAG, 1)? == [0],
        "nonzero entry flag requires an unqualified incoming caller argument"
    );
    ensure!(
        image.initial_va(ENTRY, INITIAL_SOURCE.len())? == INITIAL_SOURCE,
        "unsupported initial entry instructions"
    );
    ensure!(
        image.initial_va(JUMP, INITIAL_JUMP.len())? == INITIAL_JUMP,
        "unsupported initial forwarding jump"
    );
    Ok(Layout { source, jump, flag, patch })
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
