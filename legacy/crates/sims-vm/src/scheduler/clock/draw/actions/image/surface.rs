//!60579a's unlocked-surface path,604d93 COM-interface release order and
//!5ec4c2/604d88 deleting destructor. Platform references/free are host services.
pub trait Backend {
    fn release(&mut self, fields: &mut [u8], interface: u32) -> Result<(), String>;
    fn free(&mut self, fields: &mut [u8], surface: u32) -> Result<(), String>;
}
fn word(h: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(h[off..off + 4].try_into().unwrap())
}
fn set(h: &mut [u8], off: usize, v: u32) {
    h[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn release_interfaces(h: &mut [u8], host: &mut impl Backend) -> Result<(), String> {
    if h.len() < 0x188 {
        return Err("surface interface fields absent".into());
    }
    for off in [8, 4] {
        let interface = word(h, off);
        if interface != 0 {
            host.release(h, interface)?;
            set(h, off, 0);
        }
    }
    h[0x15c] = 0;
    Ok(())
}
pub fn close(h: &mut [u8], host: &mut impl Backend) -> Result<(), String> {
    if h.len() < 0x188 || word(h, 0x184) != 0 {
        return Err("locked surface requires its native unlock/error caller".into());
    }
    release_interfaces(h, host)
}
pub fn destroy(id: u32, h: &mut [u8], flags: u32, host: &mut impl Backend) -> Result<(), String> {
    if id == 0 || h.len() < 0x188 {
        return Err("surface destruction owner absent".into());
    }
    set(h, 0, 0x676cf8);
    release_interfaces(h, host)?;
    if flags & 1 != 0 {
        host.free(h, id)?;
    }
    Ok(())
}
