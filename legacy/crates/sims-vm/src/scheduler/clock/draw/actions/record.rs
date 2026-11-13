//!4e877f construction,4ea203 image selection and4e87b7 record-copy callers.
//! Allocator/image factories/string assignment and live action-label lookup
//! remain explicit services. Undefined constructor/padding bytes stay untouched.
use super::Record;
/// Bytes initialized by the constructor. Person/flags/padding are undefined
/// until later writers; host scratch values do not qualify those bytes.
pub const CONSTRUCTED_BYTES: u32 = 0x0fff_f0f0;
pub const BUILT_BYTES:u32 = 0x0fff_f3ff;
pub fn word(raw: &Record, offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
fn set(raw: &mut Record, offset: usize, value: u32) {
    raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
pub trait Strings {
    fn allocate(&mut self, bytes: u32) -> Result<u32, String>;
    fn write_nul(&mut self, pointer: u32) -> Result<(), String>;
}
pub trait Caption: Strings {
    fn read(&self, pointer: u32, bytes: u32) -> Result<Vec<u8>, String>;
    fn write(&mut self, pointer: u32, bytes: &[u8]) -> Result<(), String>;
    fn free(&mut self, pointer: u32, bytes: u32) -> Result<(), String>;
    /// The header becomes visible only after the native allocation/free path
    /// commits it; hosts may inspect the current header at those boundaries.
    fn header(&mut self, _header: [u32; 3]) {}
}
pub enum Text<'a> {
    Same,
    Bytes(&'a [u8]),
}
///5d4b12/4316ca/412d17: byte string assignment, with native length-based
/// growth and prefix writes before allocation. Allocator internals/SEH remain
/// host services. Unwritten reserved bytes retain their allocation contents.
pub fn assign_caption(
    header: &mut [u32; 3],
    source: Text<'_>,
    host: &mut impl Caption,
) -> Result<(), String> {
    let Text::Bytes(source) = source else {
        return Ok(());
    };
    let [begin, end, limit] = *header;
    let old = end.checked_sub(begin).ok_or("caption length underflow")?;
    let capacity = limit
        .checked_sub(begin)
        .filter(|n| *n > old)
        .ok_or("caption capacity invalid")?;
    let length = u32::try_from(source.len()).map_err(|_| "caption exceeds32-bit length")?;
    if length > 0xffff_fffe {
        return Err("caption exceeds native length limit".into());
    }
    let prefix = old.min(length);
    host.write(begin, &source[..prefix as usize])?;
    if length < capacity {
        if length > old {
            host.write(begin + old, &source[old as usize..])?;
        }
        host.write_nul(begin + length)?;
        header[1] = begin + length;
    } else {
        let capacity = old
            .checked_add(old.max(length - old))
            .and_then(|n| n.checked_add(1))
            .ok_or("caption growth overflow")?;
        host.header(*header);
        let next = host.allocate(capacity)?;
        if next == 0 {
            return Err("caption allocator returned null".into());
        }
        let prefix = host.read(begin, old)?;
        host.write(next, &prefix)?;
        host.write(next + old, &source[old as usize..])?;
        host.write_nul(next + length)?;
        host.free(begin, limit - begin)?;
        *header = [next, next + length, next + capacity];
    }
    host.header(*header);
    Ok(())
}
pub trait Images {
    ///5a5f4d/79 retain incoming before releasing previous, even if identical.
    fn assign(&mut self, slot: &mut u32, image: u32) -> Result<(), String>;
    fn release(&mut self, image: u32) -> Result<(), String>;
}
pub trait Copy: Strings + Images {
    ///41f167 reads/updates the actual action, not the source record caption.
    fn action_label(&mut self, node: u32) -> Result<Vec<u8>, String>;
    fn caption(&mut self, header: &mut [u32; 3], bytes: &[u8]) -> Result<(), String>;
}
pub fn construct(raw: &mut Record, host: &mut impl Strings) -> Result<(), String> {
    set(raw, 4, 0);
    set(raw, 12, 0);
    for off in [16, 20, 24] {
        set(raw, off, 0);
    }
    let allocation = host.allocate(8)?;
    if allocation == 0 {
        return Err("caption allocator returned null instead of its failure boundary".into());
    }
    set(raw, 16, allocation);
    set(raw, 20, allocation);
    set(raw, 24, allocation.wrapping_add(8));
    host.write_nul(allocation)
}
pub fn copy(destination: &mut Record, source: &Record, host: &mut impl Copy) -> Result<(), String> {
    construct(destination, host)?;
    set(destination, 0, word(source, 0));
    let mut image = word(destination, 4);
    host.assign(&mut image, word(source, 4))?;
    set(destination, 4, image);
    destination[8] = source[8];
    destination[9] = source[9];
    set(destination, 12, word(source, 12));
    let label = host.action_label(word(destination, 12))?;
    let mut header = [
        word(destination, 16),
        word(destination, 20),
        word(destination, 24),
    ];
    host.caption(&mut header, &label)?;
    for (i, value) in header.into_iter().enumerate() {
        set(destination, 16 + i * 4, value);
    }
    Ok(())
}
pub trait Build: Images {
    fn icon_owner(&self, node: u32) -> Result<u32, String>;
    fn kind(&self, object: u32) -> Result<u16, String>;
    fn resource(&self, object: u32) -> Result<u32, String>;
    fn guid(&self, resource: u32) -> Result<u32, String>;
    fn cache_index(&self, guid: u32) -> i32;
    fn portrait(&mut self, index: i32, temporary: &mut u32) -> Result<bool, String>;
    fn master(&mut self, resource: u32) -> Result<Option<u32>, String>;
    fn descriptor(&self, master: u32) -> Result<Option<u32>, String>;
    fn thumbnail(&mut self, descriptor: u32, temporary: &mut u32) -> Result<bool, String>;
    fn active_counter(&self, person: u32) -> Result<u16, String>;
}
pub fn build(
    raw: &mut Record,
    index: u32,
    person: u32,
    node: u32,
    current: u8,
    host: &mut impl Build,
) -> Result<bool, String> {
    let object = host.icon_owner(node)?;
    if object == 0 {
        return Ok(false);
    }
    let kind = host.kind(object)?;
    let resource = host.resource(object)?;
    if kind == 2 {
        let guid = host.guid(resource)?;
        let index = host.cache_index(guid);
        let mut temporary = 0;
        if host.portrait(index, &mut temporary)? {
            let mut image = word(raw, 4);
            host.assign(&mut image, temporary)?;
            set(raw, 4, image);
        }
        if temporary != 0 {
            host.release(temporary)?;
        }
    } else {
        let Some(master) = host.master(resource)? else {
            return Ok(false);
        };
        if let Some(descriptor) = host.descriptor(master)? {
            let mut temporary = 0;
            // Native ignores4ccaa0's status and copies the wrapper regardless.
            host.thumbnail(descriptor, &mut temporary)?;
            let mut image = word(raw, 4);
            host.assign(&mut image, temporary)?;
            set(raw, 4, image);
            if temporary != 0 {
                host.release(temporary)?;
            }
        }
    }
    raw[9] = current;
    raw[8] = if current == 0 {
        1
    } else {
        u8::from(host.active_counter(person)? == 0)
    };
    set(raw, 12, node);
    set(raw, 0, index);
    Ok(true)
}
