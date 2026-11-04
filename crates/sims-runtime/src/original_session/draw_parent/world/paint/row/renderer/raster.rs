//! Frozen-B palette/particle/pixel inputs; never C8 expected output injection.
use super::*;
use sims_sim::lot_presenter::terrain_raster::{Data, ParticleColumn};
pub fn import(source: &[u8], color: &[u8], depth: &[u8], owner: &[u8]) -> Result<Data> {
    ensure!(
        sims_fixtures::digest(source)
            == "eaa1e23a01b976d440e5d8319769691028683f732a72c766d16bbf675777a47a",
        "raster source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        v["tick"] == 89073 && v["executes_target_code"] == false,
        "raster source epoch differs"
    );
    ensure!(
        color.len() == 960000
            && depth.len() == 960000
            && sims_fixtures::digest(color)
                == "29088ba6368103c7b7c9bc3c86fdd1657b566401a3c4207f5c1d439aebe517fd"
            && sims_fixtures::digest(depth)
                == "c9b18600471e119f2d3015f3d5681a70c16e25308df86df3095c6e6a00bd3811",
        "baseline raster buffers differ"
    );
    let m = Memory::new(&v)?;
    let base = m.read(0x0a98d928, 0x3d8)?;
    ensure!(
        base[0xe0..0x360] == owner[0xe0..0x360],
        "raster descriptors changed before first binding"
    );
    let shades = word(&base, 0x7c) as usize;
    let colors = word(&base, 0x4c) as usize;
    let pointers = m.read(word(&base, 0x70), shades * 4)?;
    let palette = pointers
        .chunks_exact(4)
        .map(|p| m.read(word(p, 0), colors * 16))
        .collect::<Result<Vec<_>>>()?;
    let particles = (0..8)
        .map(|i| {
            let lo = word(&base, 0xe0 + i * 20);
            let hi = word(&base, 0xe4 + i * 20);
            Ok(ParticleColumn {
                begin: lo,
                records: m.read(lo, hi.wrapping_sub(lo) as usize)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let data = Data {
        colors,
        palette,
        particles,
        color: color.to_vec(),
        depth: depth.to_vec(),
        color_pitch: 1600,
        depth_pitch: 1600,
    };
    data.validate(owner).map_err(anyhow::Error::msg)?;
    Ok(data)
}
