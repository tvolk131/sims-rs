//! B terrain/variation inputs; live covering and pool queries borrow the VM lot.
//! The map supplement is not a claim about its unimplemented update lifecycle.
use super::*;
use sims_vm::scheduler::clock::draw::ground::visible::{Frame, Reply, Request};
const SOURCE: &str = "7108df1df3ec25ccc3455683327afccbc1e5e55be63205e6e989b3ace9fdce73";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    width: usize,
    height: usize,
    signed: Vec<i8>,
    variation_width: usize,
    variation_height: usize,
    variation: Vec<u16>,
    particle_heights: [i32; 4],
    pool_present: bool,
}
impl Source {
    pub fn particle_height(&self, zoom: u32) -> Result<i32> {
        ensure!((1..=3).contains(&zoom), "terrain particle zoom differs");
        Ok(self.particle_heights[zoom as usize])
    }
    pub fn import(input: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == SOURCE,
            "terrain preparation source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "terrain preparation source epoch differs"
        );
        let m = Memory::new(&v)?;
        let world = word(&m.read(0x6e552c, 4)?, 0);
        let lr = m.read(world, 0x88)?;
        let signed_header = m.read(word(&lr, 0x1c), 32)?;
        let width = word(&signed_header, 4) as usize;
        let height = word(&signed_header, 8) as usize;
        let pointers = m.read(word(&signed_header, 0xc), width * 4)?;
        let mut signed = vec![];
        for p in pointers.chunks_exact(4) {
            signed.extend(m.read(word(p, 0), height)?.into_iter().map(|v| v as i8));
        }
        let renderer = m.read(0x0a98d928, 0x3d8)?;
        let header = m.read(word(&renderer, 0x68), 32)?;
        let vw = word(&header, 4) as usize;
        let vh = word(&header, 8) as usize;
        let pointers = m.read(word(&header, 0xc), vw * 4)?;
        let mut variation = vec![];
        for p in pointers.chunks_exact(4) {
            variation.extend(
                m.read(word(p, 0), vh * 2)?
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes(b.try_into().unwrap())),
            );
        }
        let table = m.read(0x6c628c, 16)?;
        let s = Self {
            source: SOURCE.into(),
            width,
            height,
            signed,
            variation_width: vw,
            variation_height: vh,
            variation,
            particle_heights: std::array::from_fn(|i| word(&table, i * 4) as i32),
            pool_present: word(&lr, 0x60) != 0,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && (1..=64).contains(&self.width)
                && (1..=64).contains(&self.height)
                && self.signed.len() == self.width * self.height
                && (1..=64).contains(&self.variation_width)
                && (1..=64).contains(&self.variation_height)
                && self.variation.len() == self.variation_width * self.variation_height,
            "terrain map owner differs"
        );
        Ok(())
    }
    pub fn advance(
        &self,
        f: &mut Frame,
        raw: &mut [u8],
        sim: &sims_sim::Simulation,
        rotation: u8,
        active: usize,
        zoom: u32,
    ) -> Result<bool> {
        self.validate()?;
        f.validate(raw).map_err(anyhow::Error::msg)?;
        if let Some(request) = f.pending() {
            let map = sims_sim::lot_presenter::terrain_shade::Map {
                width: self.width,
                height: self.height,
                rotation,
                cells: &self.signed,
            };
            let reply = match request {
                Request::Shade { point } => Reply::Shade(
                    map.corner([point[0] as i8, point[1] as i8, 1], word(raw, 0x7c) as i32)
                        .map_err(anyhow::Error::msg)?,
                ),
                Request::Terrain { point } => {
                    Reply::Terrain(map.sample(point).map_err(anyhow::Error::msg)?)
                }
                Request::Variation { point: [x, y] } => {
                    ensure!(
                        x >= 0
                            && y >= 0
                            && (x as usize) < self.variation_width
                            && (y as usize) < self.variation_height,
                        "terrain variation outside retained grid"
                    );
                    Reply::Variation(
                        self.variation[x as usize * self.variation_height + y as usize],
                    )
                }
                Request::Covering { point } | Request::PoolInclusive { point } => {
                    let lot = sim
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("terrain live lot absent")?;
                    let [x, y, z] =
                        sims_sim::lot_presenter::terrain_shade::inverse(point, rotation)
                            .map_err(anyhow::Error::msg)?;
                    ensure!(
                        x >= 0 && y >= 0 && (x as usize) < lot.size && (y as usize) < lot.size,
                        "terrain live query outside retained grid"
                    );
                    let i = x as usize * lot.size + y as usize;
                    match request {
                        Request::Covering { .. } => Reply::Covering(
                            lot.floor(i32::from(z))
                                .context("terrain covering floor absent")?
                                .source
                                .coverings[i],
                        ),
                        _ => {
                            ensure!(
                                self.pool_present,
                                "terrain inclusive pool requires its actual grid"
                            );
                            Reply::PoolInclusive(z == 1 && lot.pool[i] <= 16)
                        }
                    }
                }
                Request::WallFlags { point } => {
                    let lot = sim
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("terrain live lot absent")?;
                    let floor = lot
                        .floor(i32::from(point[2]))
                        .context("terrain wall floor absent")?;
                    Reply::WallFlags(
                        sims_sim::lot_presenter::terrain_walls::segments(
                            lot.size,
                            &floor.source.walls,
                            point,
                            rotation,
                        )
                        .map_err(anyhow::Error::msg)?,
                    )
                }
                Request::Snow => return Ok(false),
            };
            f.reply(request, reply).map_err(anyhow::Error::msg)?;
            return Ok(true);
        }
        ensure!(zoom < 4, "terrain particle zoom differs");
        f.advance(raw, active as i32, self.particle_heights[zoom as usize])
            .map_err(anyhow::Error::msg)
    }
}
