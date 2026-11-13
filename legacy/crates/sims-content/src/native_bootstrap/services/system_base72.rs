//! Concrete resources for the first Base72 child in a complete system1240.
//! This binds explicit source images to one logical heap and one retained Rust
//! lock. It grants no provider return, native synchronization or alpha gate.
use super::{BootstrapServices, Capability, unsupported};
use crate::native_bootstrap::memory::{Identity, Kind, Memory};
use sims_sim::{
    native_application::{InitializedSection, SectionRequest},
    native_system::base72::{Prefix, Request},
    native_world_children::Owner,
    saved_matrix::{Allocated, Stop},
};
use std::sync::{Arc, Mutex};

const PARENT_BYTES: usize = 1240;
const SECTION_OFFSET: usize = 60;
const SECTION_BYTES: u32 = 24;
const DEBUG_BYTES: u32 = 32;

pub(super) struct Binding {
    heap: Identity,
    pub(super) request: Request,
    pub(super) returned: InitializedSection,
    pub(super) parent_image: Owner,
    pub(super) caller_image: Owner,
    lock: Arc<Mutex<()>>,
    acknowledged: bool,
}

fn end(owner: &Owner) -> Result<u64, Stop> {
    if owner.token == 0 || owner.data.is_empty() {
        return Err(unsupported("system Base72 needs a complete nonnull owner"));
    }
    owner
        .token
        .checked_add(owner.data.len() as u64)
        .filter(|end| *end <= 1_u64 << 32)
        .ok_or_else(|| unsupported("system Base72 owner exceeds native width"))
}

fn separate(first: &Owner, second: &Owner) -> Result<(), Stop> {
    let first_end = end(first)?;
    let second_end = end(second)?;
    if first.token < second_end && second.token < first_end {
        return Err(unsupported("system Base72 complete owners overlap"));
    }
    Ok(())
}

// Exact map lookup is deliberate: a containing Kind::Owner view alone would
// also admit an embedded/shifted image or an Application324 allocation.
fn live_image(memory: &Memory, image: &Owner, kind: Kind) -> Result<(), Stop> {
    end(image)?;
    if memory.allocation_kind(image.token)? != kind
        || memory.allocation_bytes(image.token)? != image.data
    {
        return Err(unsupported(
            "system Base72 allocation role, complete extent or full image changed",
        ));
    }
    Ok(())
}

fn copy_owner(image: &Owner) -> Result<Owner, Stop> {
    let mut data = Vec::new();
    data.try_reserve_exact(image.data.len()).map_err(|_| {
        Stop::Unavailable("system Base72 image storage could not be reserved".into())
    })?;
    data.extend_from_slice(&image.data);
    Ok(Owner {
        token: image.token,
        data,
    })
}

fn checked_request(request: &Request) -> Result<SectionRequest, Stop> {
    // The cursor alone authenticates the source stores and seals these images.
    request.validate()?;
    let parent = request.parent();
    let initial_parent = request.initial_parent();
    let caller = request.caller_frame();
    let initial_caller = request.initial_caller_frame();
    if parent.data.len() != PARENT_BYTES
        || initial_parent.data.len() != PARENT_BYTES
        || parent.token != initial_parent.token
        || caller.token != initial_caller.token
        || caller.data.len() != initial_caller.data.len()
    {
        return Err(unsupported(
            "system Base72 requires exact complete initial owners",
        ));
    }
    separate(parent, caller)?;
    separate(initial_parent, initial_caller)?;
    let first = request
        .ebp()
        .checked_sub(0x10)
        .ok_or_else(|| unsupported("system Base72 caller frame underflows"))?;
    let last = request
        .ebp()
        .checked_add(0x0c)
        .filter(|end| *end <= 1_u64 << 32)
        .ok_or_else(|| unsupported("system Base72 caller frame overflows"))?;
    if first < caller.token || last > end(caller)? {
        return Err(unsupported("system Base72 caller frame is incomplete"));
    }
    let argument = (request.ebp() + 8 - caller.token) as usize;
    if initial_caller.data[argument..argument + 4] != [0; 4]
        || caller.data[argument..argument + 4] != [0; 4]
    {
        return Err(unsupported(
            "system Base72 supports only the first caller DWORD0",
        ));
    }
    let section = request.section_request();
    if section.instruction != 0x5d91bc
        || section.import_slot != 0x666114
        || section.region.token != parent.token + SECTION_OFFSET as u64
        || section.region.data.len() != SECTION_BYTES as usize
        || section.region.data != parent.data[SECTION_OFFSET..SECTION_OFFSET + 24]
    {
        return Err(unsupported(
            "system Base72 section is not exact parent60..84",
        ));
    }
    Ok(section)
}

impl Binding {
    pub(super) fn require_live(&self, memory: &Memory) -> Result<(), Stop> {
        if !memory.has_identity(&self.heap) {
            return Err(unsupported(
                "system Base72 section belongs to another heap lifetime",
            ));
        }
        let section = checked_request(&self.request)?;
        self.returned.validate(&section)?;
        live_image(memory, &self.parent_image, Kind::Owner)?;
        live_image(memory, &self.caller_image, Kind::Owner)?;
        let debug = self
            .returned
            .debug_info
            .as_ref()
            .ok_or_else(|| unsupported("system Base72 lost its actual debug allocation"))?;
        if debug.data.len() != DEBUG_BYTES as usize {
            return Err(unsupported("system Base72 debug extent changed"));
        }
        separate(&self.parent_image, debug)?;
        separate(&self.caller_image, debug)?;
        live_image(memory, debug, Kind::SystemSectionDebug)?;
        if self.parent_image.data[SECTION_OFFSET..SECTION_OFFSET + 24] != self.returned.region.data
        {
            return Err(unsupported("system Base72 retained section image changed"));
        }
        Ok(())
    }

    pub(super) fn section<'b>(
        &'b self,
        memory: &Memory,
        address: u64,
        bytes: u32,
    ) -> Result<&'b Arc<Mutex<()>>, Stop> {
        if address != self.returned.region.token || bytes != SECTION_BYTES {
            return Err(unsupported(
                "system Base72 lookup has another section extent",
            ));
        }
        self.require_live(memory)?;
        Ok(&self.lock)
    }

    pub(super) fn debug_info<'b>(
        &'b self,
        memory: &Memory,
        address: u64,
        bytes: u32,
    ) -> Result<Option<&'b Owner>, Stop> {
        self.section(memory, address, bytes)?;
        Ok(self.returned.debug_info.as_ref())
    }
}

pub(super) fn initialize(
    services: &mut BootstrapServices<'_>,
    request: &Request,
) -> Result<InitializedSection, Stop> {
    super::system_factory::require_origin(services, request)?;
    let section = checked_request(request)?;
    for capability in services.capabilities.values() {
        match capability {
            Capability::SystemBase72(binding)
                if binding.returned.region.token == section.region.token =>
            {
                if !binding.request.same_call(request) {
                    return Err(unsupported(
                        "system Base72 address belongs to another source call",
                    ));
                }
                binding.require_live(&services.memory)?;
                // Acknowledged images may be newer than the import snapshots.
                // A retry returns the retained result without resetting either.
                return Ok(binding.returned.clone());
            }
            Capability::Section(binding) if binding.address == section.region.token => {
                return Err(unsupported(
                    "system Base72 address already has another section role",
                ));
            }
            _ => (),
        }
    }
    // The parent factory origin check above supplies the lifetime provenance
    // that byte-identical initial owners cannot establish on their own.
    live_image(&services.memory, request.initial_parent(), Kind::Owner)?;
    live_image(
        &services.memory,
        request.initial_caller_frame(),
        Kind::Owner,
    )?;

    // Reserve the capability and all fallible full-image storage before the
    // allocator effect. A quota failure never becomes a native null return.
    let token = services.next_capability()?;
    let held_request = request.clone();
    let mut parent_image = copy_owner(request.parent())?;
    let caller_image = copy_owner(request.caller_frame())?;
    let heap = services.memory.identity();
    let lock = Arc::new(Mutex::new(()));
    let Allocated::Bytes {
        token: debug_token,
        data,
    } = services
        .memory
        .allocate(Kind::SystemSectionDebug, DEBUG_BYTES)?
    else {
        unreachable!("the explicit logical heap never manufactures native null")
    };

    // No Result::Err may follow the successful allocation. Fresh monotonic
    // storage in this exclusively borrowed heap is disjoint from BOTH complete
    // owners, native-width and exactly32 bytes; the prechecked import is24.
    let debug_input = Owner {
        token: debug_token,
        data,
    };
    let returned =
        InitializedSection::from_debug_info(&section, Some(&debug_input), u64::from(token))
            .expect("checked request and fresh same-heap debug owner satisfy section layout");
    separate(&parent_image, &debug_input)
        .expect("fresh heap allocation cannot overlap the complete parent");
    separate(&caller_image, &debug_input)
        .expect("fresh heap allocation cannot overlap the complete caller");
    parent_image.data[SECTION_OFFSET..SECTION_OFFSET + 24].copy_from_slice(&returned.region.data);
    services
        .memory
        .synchronize_owner(&parent_image)
        .expect("prechecked complete parent is still live in the exclusive heap");
    services
        .memory
        .synchronize_owner(&caller_image)
        .expect("prechecked complete caller is still live in the exclusive heap");
    services
        .memory
        .synchronize_owner(returned.debug_info.as_ref().expect("actual debug owner"))
        .expect("fresh complete debug owner is still live in the exclusive heap");
    let result = returned.clone();
    services.commit(
        token,
        Capability::SystemBase72(Binding {
            heap,
            request: held_request,
            returned,
            parent_image,
            caller_image,
            lock,
            acknowledged: false,
        }),
    );
    Ok(result)
}

pub(super) fn acknowledge(
    services: &mut BootstrapServices<'_>,
    prefix: &Prefix,
) -> Result<(), Stop> {
    prefix.validate()?;
    checked_request(prefix.request())?;
    let token = u32::try_from(prefix.section().capability)
        .map_err(|_| unsupported("system Base72 capability exceeds native width"))?;
    let Some(Capability::SystemBase72(binding)) = services.capabilities.get(&token) else {
        return Err(unsupported(
            "system Base72 prefix has no issued section capability",
        ));
    };
    if !binding.request.same_call(prefix.request()) || binding.returned != *prefix.section() {
        return Err(unsupported(
            "system Base72 prefix belongs to another source call or return",
        ));
    }
    binding.require_live(&services.memory)?;
    if prefix.parent().token != binding.parent_image.token
        || prefix.parent().data.len() != PARENT_BYTES
        || prefix.caller_frame().token != binding.caller_image.token
        || prefix.caller_frame().data.len() != binding.caller_image.data.len()
        || prefix.parent().data[SECTION_OFFSET..SECTION_OFFSET + 24] != binding.returned.region.data
    {
        return Err(unsupported(
            "system Base72 prefix changed its complete owner extents",
        ));
    }
    // An old normal-return prefix is historical after acknowledgement.
    // Later root source cuts may advance the same full images; never rewind.
    if binding.acknowledged {
        return Ok(());
    }
    let parent_image = copy_owner(prefix.parent())?;
    let caller_image = copy_owner(prefix.caller_frame())?;
    // Every fallible check/reservation precedes the first synchronized byte.
    services
        .memory
        .synchronize_owner(&parent_image)
        .expect("authenticated prefix retains the same complete live parent");
    services
        .memory
        .synchronize_owner(&caller_image)
        .expect("authenticated prefix retains the same complete live caller");
    let Some(Capability::SystemBase72(binding)) = services.capabilities.get_mut(&token) else {
        unreachable!("exclusive acknowledgement retains the checked capability")
    };
    binding.parent_image = parent_image;
    binding.caller_image = caller_image;
    binding.acknowledged = true;
    Ok(())
}
