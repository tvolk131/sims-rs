//!4e88be dirty getter,4d3f42 append,4d3fe6 whole-range erase and the
//!4cb260 idle branch. Capacity growth and active tweens stay explicit.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Append {
    Complete,
    Growth { position: u32, value: u32 },
}
pub fn caption_dirty(flags: u32) -> u32 {
    (flags >> 7) & 1
}
pub fn validate(header: &[u32; 3], backing: &[u8]) -> Result<(), String> {
    let [begin, end, capacity] = *header;
    if begin > end
        || end > capacity
        || (end - begin) % 4 != 0
        || (capacity - begin) % 4 != 0
        || capacity - begin > 65536
        || backing.len() != (capacity - begin) as usize
        || begin == 0 && capacity != 0
    {
        return Err("pointer-vector allocation extent differs".into());
    }
    Ok(())
}
pub fn append(header: &mut [u32; 3], backing: &mut [u8], value: u32) -> Result<Append, String> {
    validate(header, backing)?;
    if header[1] == header[2] {
        return Ok(Append::Growth {
            position: header[1],
            value,
        });
    }
    let at = (header[1] - header[0]) as usize;
    backing[at..at + 4].copy_from_slice(&value.to_le_bytes());
    header[1] = header[1].wrapping_add(4);
    Ok(Append::Complete)
}
pub fn clear(header: &mut [u32; 3], backing: &[u8]) -> Result<(), String> {
    validate(header, backing)?;
    header[1] = header[0];
    Ok(())
}
pub fn idle_advance(header: &[u8]) -> Result<bool, String> {
    if header.len() < 0x124 {
        return Err("action animation window absent".into());
    }
    Ok(header[0xf0] == 0)
}
