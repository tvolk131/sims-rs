//!5dfc94/5dfcb0/5dec72 bitmap destruction. Platform virtuals, recursive image
//! release, string destruction and deallocation remain explicit backend calls.
pub trait Backend {
    fn virtual_call(
        &mut self,
        header: &mut [u8],
        owner: u32,
        slot: u16,
        arguments: &[u32],
    ) -> Result<(), String>;
    fn release_image(&mut self, header: &mut [u8], image: u32) -> Result<(), String>;
    fn destroy_string(&mut self, header: &mut [u8], string: u32) -> Result<(), String>;
    fn free(&mut self, header: &mut [u8], owner: u32) -> Result<(), String>;
}
fn word(h: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(h[off..off + 4].try_into().unwrap())
}
fn set(h: &mut [u8], off: usize, v: u32) {
    h[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn cleanup(h: &mut [u8], host: &mut impl Backend) -> Result<(), String> {
    if h.len() < 100 {
        return Err("bitmap cleanup header absent".into());
    }
    let surface = word(h, 0x48);
    if surface != 0 {
        host.virtual_call(h, surface, 4, &[])?;
        let surface = word(h, 0x48);
        if surface != 0 {
            host.virtual_call(h, surface, 0, &[1])?;
        }
        set(h, 0x48, 0);
    }
    let palette = word(h, 0x4c);
    if palette != 0 {
        host.virtual_call(h, palette, 0, &[1])?;
        set(h, 0x4c, 0);
    }
    for off in [0x5c, 0x60] {
        let image = word(h, off);
        if image != 0 {
            host.release_image(h, image)?;
            set(h, off, 0);
        }
    }
    Ok(())
}
pub fn destroy(
    image: u32,
    h: &mut [u8],
    flags: u32,
    host: &mut impl Backend,
) -> Result<(), String> {
    if image == 0 || h.len() < 100 {
        return Err("bitmap destruction owner absent".into());
    }
    set(h, 0, 0x673d4c);
    let palette = word(h, 0x4c);
    if palette != 0 {
        host.virtual_call(h, palette, 0, &[1])?;
        set(h, 0x4c, 0);
    }
    cleanup(h, host)?;
    let buffer = word(h, 0x10);
    if buffer != 0 {
        host.virtual_call(h, buffer, 0, &[1])?;
    }
    let string = word(h, 0x2c);
    if string != 0 {
        host.destroy_string(h, string)?;
        host.free(h, string)?;
    }
    if flags & 1 != 0 {
        host.free(h, image)?;
    }
    Ok(())
}
