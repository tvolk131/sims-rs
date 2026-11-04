//! Original integer sprite-depth operations (4ca62e/4ca78f and49a5d3).
//! This is the reference blitter for already decoded palette/depth rows. The
//! scene renderer still has its approximate depth mapping; it cannot use this
//! path until its sprite-position/lookup producer uses the same depth space.
use anyhow::{Result, ensure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table(pub [u16; 255]);
impl Table {
    pub fn new(scale_bits: u32) -> Self {
        Self(sims_sim::lot_presenter::camera_command::depth_table(
            scale_bits,
        ))
    }
    /// Native signed32 add/compare, followed by a low16 store. Byte255 skips
    /// both color and depth, regardless of the palette index. Equal depth wins.
    pub fn sample(&self, base: i32, byte: u8, previous: u16) -> Option<u16> {
        let offset = *self.0.get(usize::from(byte))? as i16;
        let candidate = base.wrapping_add(i32::from(offset));
        (candidate <= i32::from(previous)).then_some(candidate as u16)
    }
}

/// 49a5d3: the selected zoom's lookup is supplied by the caller. Bounds checks
/// reject unavailable memory, rather than substituting a guessed base depth.
pub fn base_depth(
    lookup: &[i16],
    zoom: u32,
    screen_y: i32,
    scan_y: i32,
    anchor: i32,
) -> Result<i32> {
    ensure!((1..=3).contains(&zoom), "sprite zoom unavailable");
    let index = screen_y.wrapping_add(8 << zoom).wrapping_sub(scan_y);
    let offset = lookup
        .get(usize::try_from(index).map_err(|_| anyhow::anyhow!("negative depth lookup"))?)
        .ok_or_else(|| anyhow::anyhow!("sprite depth lookup unavailable"))?;
    Ok(anchor.wrapping_add(i32::from(*offset)))
}

pub struct Rows<'a> {
    pub width: usize,
    pub height: usize,
    pub colors: &'a [u8],
    /// Mutable decoder scratch: original blits consume present depth rows.
    pub depths: &'a mut [u8],
    /// Native decoder AL must be exactly1; other values skip a color row.
    pub depth_results: &'a [u8],
}
pub struct Surface<'a> {
    pub colors: &'a mut [u16],
    pub depths: &'a mut [u16],
    pub color_pitch: usize,
    pub depth_pitch: usize,
}

/// Blit into the already-positioned destination. Clip amounts are left, top,
/// right, bottom. The outer sprite-position/surface clipping caller is separate.
pub fn blit(
    table: &Table,
    base: i32,
    rows: Rows<'_>,
    palette: &[u16; 256],
    surface: Surface<'_>,
    clip: [usize; 4],
    flip: bool,
) -> Result<()> {
    let count = rows
        .width
        .checked_mul(rows.height)
        .ok_or_else(|| anyhow::anyhow!("sprite size overflow"))?;
    let [left, top, right, bottom] = clip;
    ensure!(
        left <= rows.width && right <= rows.width && top <= rows.height && bottom <= rows.height,
        "invalid sprite clip"
    );
    ensure!(
        rows.colors.len() >= count
            && rows.depths.len() >= count
            && rows.depth_results.len() >= rows.height,
        "decoded sprite rows unavailable"
    );
    for (pitch, len) in [
        (surface.color_pitch, surface.colors.len()),
        (surface.depth_pitch, surface.depths.len()),
    ] {
        ensure!(
            pitch >= rows.width
                && pitch
                    .checked_mul(rows.height)
                    .is_some_and(|size| size <= len),
            "sprite destination unavailable"
        );
    }
    for y in top..rows.height - bottom {
        if rows.depth_results[y] != 1 {
            continue;
        }
        for x in left..rows.width - right {
            let source_x = if flip { rows.width - x - 1 } else { x };
            let source = y * rows.width + source_x;
            let target = y * surface.depth_pitch + x;
            if let Some(depth) = table.sample(base, rows.depths[source], surface.depths[target]) {
                surface.colors[y * surface.color_pitch + x] =
                    palette[usize::from(rows.colors[source])];
                surface.depths[target] = depth;
            }
        }
        // Includes clipped pixels. Skipped rows retain their decoder scratch.
        rows.depths[y * rows.width..(y + 1) * rows.width].fill(255);
    }
    Ok(())
}
