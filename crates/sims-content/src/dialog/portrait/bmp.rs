//! 4b7d93's 16-bit surface to 24-bit BMP conversion. Surface locking belongs
//! to the caller. Allocation contents are explicit: native row padding is not
//! initialized by the exporter. Rust-owned output normally supplies zeroes.
use super::resize::{Format, Source};

pub fn encode(
    format: Format,
    source: Source<'_>,
    crop: Option<[i32; 4]>,
    reduce_green: bool,
    allocation_fill: u8,
) -> Result<Vec<u8>, String> {
    if source.width == 0
        || source.height == 0
        || source.width > i32::MAX as usize
        || source.height > i32::MAX as usize
        || source.stride < source.width
        || source
            .stride
            .checked_mul(source.height)
            .is_none_or(|n| n > source.pixels.len())
    {
        return Err("invalid BMP source storage".into());
    }
    let [left, top, right, bottom] =
        crop.unwrap_or([0, 0, source.width as i32, source.height as i32]);
    if left < 0
        || top < 0
        || right <= left
        || bottom <= top
        || right as usize > source.width
        || bottom as usize > source.height
    {
        return Err("BMP crop is outside the source surface".into());
    }
    let width = (right - left) as usize;
    let height = (bottom - top) as usize;
    let stride = width
        .checked_mul(3)
        .and_then(|n| n.checked_add(3))
        .ok_or("BMP stride overflow")?
        & !3;
    let size = stride
        .checked_mul(height)
        .and_then(|n| n.checked_add(54))
        .filter(|&n| n <= 128 * 1024 * 1024)
        .ok_or("BMP output exceeds 128 MiB")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "BMP allocation failed")?;
    bytes.resize(size, allocation_fill);
    bytes[..54].fill(0);
    bytes[..2].copy_from_slice(b"BM");
    // Native leaves both file size and image size zero, including on success.
    bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
    bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height {
        let start = (bottom as usize - 1 - y) * source.stride + left as usize;
        for x in 0..width {
            let pixel = source.pixels[start + x];
            let [r, g, b] = if format == Format::Rgb565 && reduce_green {
                Format::Rgb555.decode((pixel >> 1 & 0x7fe0) | (pixel & 0x1f))
            } else {
                format.decode(pixel)
            };
            let offset = 54 + y * stride + x * 3;
            bytes[offset..offset + 3].copy_from_slice(&[b, g, r]);
        }
    }
    Ok(bytes)
}
