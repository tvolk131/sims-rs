//! Retained B inputs for the native single-gradient sprite caller.
//! Bitmap loading and two-gradient services remain separate unsupported paths.
use super::*;
mod caller;
pub(super) use caller::{Cursor, Draw};
pub(super) const SOURCE: &str = "1eba49dae2a1412ee626552723024e281cb2c7cb08f13b158c180d7b2d70cd56";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    owner: Vec<u8>,
    header_pointer: u32,
    header: Vec<u8>,
    bitmap: Vec<u8>,
    tables: Vec<u8>,
    shift: u32,
    epsilon: u32,
    flags: Vec<u8>,
    palettes: BTreeMap<u32, (u32, Vec<u8>)>,
}
impl Source {
    pub fn import(input: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == SOURCE,
            "gradient B source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "gradient source epoch differs"
        );
        let m = Memory::new(&v)?;
        let owner = m.read(0x6f01a0, 0x70)?;
        let header_pointer = word(&owner, 0x54);
        let header = m.read(header_pointer, 16)?;
        let side = word(&header, 4) as usize;
        ensure!(
            side > 0 && side <= 65536 && side % 2 == 0,
            "gradient bitmap side invalid"
        );
        let bitmap = m.read(
            word(&header, 0),
            side.checked_mul(side / 2)
                .context("gradient bitmap size overflow")?,
        )?;
        let mut palettes = BTreeMap::new();
        for p in v["palettes"]
            .as_array()
            .context("gradient raw palettes absent")?
        {
            let image = p["image"].as_u64().context("gradient image absent")? as u32;
            let resource = p["resource"].as_u64().context("gradient resource absent")? as u32;
            let owner = word(&m.read(resource, 0x2c)?, 8);
            ensure!(p["owner"] == owner, "gradient palette owner differs");
            let rgb = m.read(owner + 0x20, 1024)?;
            ensure!(
                palettes.insert(image, (owner, rgb)).is_none(),
                "duplicate gradient image"
            );
        }
        let s = Self {
            source: SOURCE.into(),
            owner,
            header_pointer,
            header,
            bitmap,
            tables: m.read(0x6edf48, 0x10e0)?,
            shift: word(&m.read(0x6c66fc, 4)?, 0),
            epsilon: word(&m.read(0x67343c, 4)?, 0),
            flags: m.read(0x6f0180, 12)?,
            palettes,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && self.owner.len() == 0x70
                && self.header.len() == 16
                && self.header_pointer == word(&self.owner, 0x54)
                && self.tables.len() == 0x10e0
                && self.shift == 2
                && self.flags.len() == 12
                && self.palettes.len() == 9
                && self
                    .palettes
                    .values()
                    .all(|(p, b)| *p != 0 && b.len() == 1024),
            "gradient retained owners differ"
        );
        let side = word(&self.header, 4) as usize;
        let pitch = word(&self.header, 8) as usize;
        ensure!(
            side == 224
                && pitch == 32
                && word(&self.header, 0) != 0
                && self.bitmap.len() == side * (side / 2)
                && f32::from_bits(self.epsilon).is_finite()
                && f32::from_bits(self.epsilon) > 0.,
            "gradient retained bitmap/configuration differs"
        );
        Ok(())
    }
    pub fn radius(&self) -> i32 {
        ((word(&self.header, 4) / word(&self.header, 8) - 1) / 2) as i32
    }
    pub fn enabled(&self) -> bool {
        self.flags[10] != 0
    }
    pub fn needs_gradient(&self, count: u32, light: u32) -> bool {
        count != 0 && (f32::from_bits(light) - 1.).abs() > f32::from_bits(self.epsilon)
    }
    pub fn mask(&self, x: i32, y: i32) -> Result<(u32, i32, &[u8])> {
        self.validate()?;
        let side = word(&self.header, 4) as i32;
        let at = sims_sim::lot_presenter::sprite_gradient::kernel_offset(
            side,
            word(&self.header, 8) as i32,
            x,
            y,
        )
        .map_err(anyhow::Error::msg)?;
        Ok((
            word(&self.header, 0)
                .checked_add(at as u32)
                .context("gradient bitmap address overflow")?,
            side,
            self.bitmap
                .get(at..)
                .context("gradient bitmap origin outside source")?,
        ))
    }
    pub fn palette(&self, image: u32) -> Result<(u32, &[u8])> {
        let (owner, rgb) = self
            .palettes
            .get(&image)
            .context("gradient image was not resident at B")?;
        Ok((*owner + 0x20, rgb))
    }
    pub fn tables(&self) -> sims_sim::lot_presenter::sprite_gradient::Tables<'_> {
        sims_sim::lot_presenter::sprite_gradient::Tables {
            bytes: &self.tables,
            green_shift: self.shift,
        }
    }
    pub fn owner(&self) -> &[u8] {
        &self.owner
    }
    pub fn header(&self) -> (u32, &[u8]) {
        (self.header_pointer, &self.header)
    }
}
