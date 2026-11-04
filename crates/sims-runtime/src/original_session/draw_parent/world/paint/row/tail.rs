//! Frozen-B row-tail inputs; pool and covering contents are borrowed from Rust.
use super::*;
use sims_sim::lot_presenter::row_tail::{Frame as Tail, Inputs};
const SOURCE: &str = "efb04abd85f0c784afb4e99fc413855fec9ea4fcf6d93ec72323faaa3d1c5439";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    bounds: [i32; 4],
    pool_present: bool,
    active_size: i32,
    neighbors: [[i8; 3]; 3],
    floor_enabled: bool,
    width: i32,
    #[serde(default)]
    contents: Option<(String, bool)>,
}
impl Source {
    pub fn import(input: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == SOURCE,
            "row tail input differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "row tail epoch differs"
        );
        let m = Memory::new(&v)?;
        let lr = m.read(word(&m.read(0x6e552c, 4)?, 0), 0x88)?;
        let steps = m.read(0x6e486c, 18)?;
        let s = Self {
            source: SOURCE.into(),
            bounds: std::array::from_fn(|i| word(&lr, 0x70 + 4 * i) as i32),
            pool_present: word(&lr, 0x60) != 0,
            active_size: word(&lr, 0x10) as i32,
            neighbors: std::array::from_fn(|i| std::array::from_fn(|j| steps[i * 6 + j] as i8)),
            floor_enabled: m.read(0x6c724a, 12)?[1] != 0,
            width: word(&m.read(0x09e88960, 0x520)?, 0x58) as i32,
            contents: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.source == SOURCE, "row tail source owner differs");
        if let Some((s, _)) = &self.contents {
            ensure!(
                s == "7108df1df3ec25ccc3455683327afccbc1e5e55be63205e6e989b3ace9fdce73",
                "row contents provenance differs"
            );
        }
        Ok(())
    }
    pub fn bind_contents(&mut self, input: &[u8]) -> Result<()> {
        ensure!(
            self.contents.is_none()
                && sims_fixtures::digest(input)
                    == "7108df1df3ec25ccc3455683327afccbc1e5e55be63205e6e989b3ace9fdce73",
            "row contents source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        let m = Memory::new(&v)?;
        self.contents = Some((
            sims_fixtures::digest(input),
            word(&m.read(0x09e88960, 0x520)?, 0x4bc) != 0,
        ));
        self.validate()
    }
    pub fn advance(
        &self,
        f: &mut Tail,
        sim: &sims_sim::Simulation,
        rotation: u8,
        rotated: [i8; 3],
    ) -> Result<bool> {
        self.validate()?;
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("row tail lot absent")?;
        if f.pending() == Some(sims_sim::lot_presenter::row_tail::Request::OtherContents) {
            let Some((_, overlay)) = &self.contents else {
                return Ok(false);
            };
            return f
                .advance_contents(&lot.water, lot.size, *overlay)
                .map_err(anyhow::Error::msg);
        }
        let [x, y, z] = rotated;
        ensure!(
            x >= 0 && y >= 0 && (x as usize) < lot.size && (y as usize) < lot.size,
            "row tail covering outside retained grid"
        );
        let covering = lot
            .floor(i32::from(z))
            .context("row tail covering floor absent")?
            .source
            .coverings[x as usize * lot.size + y as usize];
        f.advance(Inputs {
            size: lot.size,
            active_size: self.active_size,
            rotation,
            pool: self.pool_present.then_some(lot.pool.as_slice()),
            bounds: self.bounds,
            neighbors: self.neighbors,
            covering,
            floor_enabled: self.floor_enabled,
            tile_width: self.width,
        })
        .map_err(anyhow::Error::msg)
    }
}
