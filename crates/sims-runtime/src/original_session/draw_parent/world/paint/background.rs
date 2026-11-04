//! First window background owner. Source regions, stale allocation bytes and
//! initial pixel buffers come from frozen B. No expected C8 values are imported.
use super::*;
use sims_vm::scheduler::clock::draw::{
    background::{Controls, Frame},
    regions::List,
};
const SOURCE: &str = "78d1644deacc30e55e8401658198fe118336fdcc14e58ad1f664233f6c989a60";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pixels {
    pointer: u32,
    pitch: u32,
    raw: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    regions: List,
    temporary: List,
    tail: Vec<u8>,
    flags: Vec<u8>,
    behind: u8,
    full: u8,
    pixels: Vec<Pixels>,
    frame: Frame,
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("background bytes absent")?,
    )
}
fn rect(b: &[u8]) -> [i32; 4] {
    std::array::from_fn(|i| word(b, 4 * i) as i32)
}
impl State {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "background source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "background source epoch differs"
        );
        // This owner includes two960,000-byte planes. Keep the larger union
        // budget local; individual packets still obey the original1MiB cap.
        let m = Memory::with_union_limit(&v, 2 << 20)?;
        let s = &v["background_state"];
        let list = |label: &str, pointer: u32| -> Result<List> {
            let r = s["regions"]
                .as_array()
                .context("background regions absent")?
                .iter()
                .find(|r| r["label"] == label)
                .context("background region owner absent")?;
            let h = decode(&r["header"])?;
            let b = decode(&r["backing"])?;
            ensure!(
                r["pointer"] == pointer
                    && m.read(pointer, 12)? == h
                    && m.read(word(&h, 0), b.len())? == b,
                "background vector aliases differ"
            );
            let value = List {
                header: m.read(pointer, 24)?,
                backing: b,
            };
            value.validate().map_err(anyhow::Error::msg)?;
            Ok(value)
        };
        let regions = list("background_source", 0x09e88960 + 0x500)?;
        let temporary = list("background_temporary", 0x6f00c0)?;
        let globals = decode(&s["globals"])?;
        ensure!(
            m.read(0x6f00c0, 40)? == globals,
            "background global owner differs"
        );
        let mut pixels = vec![];
        for r in s["buffers"]
            .as_array()
            .context("background pixels absent")?
        {
            let pointer = r["pointer"]
                .as_u64()
                .context("background pixel pointer absent")? as u32;
            let raw = decode(&r["raw"])?;
            ensure!(
                r["pitch"] == 1600
                    && raw.len() == 960000
                    && r["sha256"] == sims_fixtures::digest(&raw)
                    && m.read(pointer, raw.len())? == raw,
                "background pixel alias differs"
            );
            pixels.push(Pixels {
                pointer,
                pitch: 1600,
                raw,
            });
        }
        ensure!(pixels.len() == 2, "background plane coverage differs");
        let world = m.read(0x09e88960, 0x520)?;
        let state = Self {
            source: SOURCE.into(),
            regions,
            temporary,
            tail: globals[24..].to_vec(),
            flags: m.read(0x6c724c, 8)?,
            behind: m.read(0x6f00e5, 1)?[0],
            full: world[0x50],
            pixels,
            frame: Frame::new(),
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && self.flags.len() == 8
                && self.tail.len() == 16
                && self.pixels.len() == 2,
            "background owner differs"
        );
        self.regions.validate().map_err(anyhow::Error::msg)?;
        self.temporary.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.pixels
                .iter()
                .all(|p| p.pitch == 1600 && p.raw.len() == 960000),
            "background pixel extent differs"
        );
        Ok(())
    }
    pub fn pending(&self) -> Option<&sims_vm::scheduler::clock::draw::background::Request> {
        self.frame.pending()
    }
    pub fn complete(&self) -> bool {
        self.frame.complete()
    }
    pub fn advance(&mut self, damage: &List, viewport: [i32; 4], flags: u32) -> Result<bool> {
        self.validate()?;
        let c = Controls {
            enabled: self.flags[1],
            behind: self.behind,
            full: self.full,
            viewport,
            delta: [
                word(&self.temporary.header, 16) as i32,
                word(&self.temporary.header, 20) as i32,
            ],
            flags,
        };
        self.frame
            .advance(c, &self.regions, damage, &mut self.temporary)
            .map_err(anyhow::Error::msg)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        self.frame.at_native_cut(pc)
    }
    pub fn compare(&self, row: &serde_json::Value, damage: &List) -> Result<()> {
        self.validate()?;
        self.frame
            .validate(&self.regions, damage, &self.temporary)
            .map_err(anyhow::Error::msg)?;
        let pc = row["pc"].as_u64().context("background PC absent")? as u32;
        ensure!(self.at_native_cut(pc), "background cursor differs at{pc:x}");
        if row["kind"] == "background_offset" {
            ensure!(
                decode(&row["rectangle"])?
                    == self
                        .frame
                        .rectangle()
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                "background offset rectangle differs at{pc:x}"
            );
            return Ok(());
        }
        ensure!(
            row["kind"] == "world_drawing_setup" && row["tick"] == 89081,
            "background observation kind/epoch differs"
        );
        let list = |label: &str, value: &List| -> Result<()> {
            let r = row["lists"]
                .as_array()
                .context("native background lists absent")?
                .iter()
                .find(|r| r["label"] == label)
                .context("native background list absent")?;
            ensure!(
                decode(&r["raw"])? == value.header && decode(&r["backing"])? == value.backing,
                "background {label} allocation differs"
            );
            Ok(())
        };
        list("source_background", &self.regions)?;
        list("temporary_background", &self.temporary)?;
        list("world", damage)?;
        let s = &row["background_state"];
        let mut globals = self.temporary.header.clone();
        globals.extend(&self.tail);
        ensure!(
            decode(&s["globals"])? == globals && decode(&s["flags"])? == self.flags,
            "background globals/flags differ"
        );
        if pc == 0x4ad0a9 {
            let raw = decode(&row["window_stack"]["raw"])?;
            ensure!(raw.len() == 128, "background window frame extent differs");
            let (a, b) = self.frame.indices();
            ensure!(
                word(&raw, 0x7c - 0x18) == a as u32
                    && word(&raw, 0x7c - 0x28) == b as u32
                    && word(&raw, 0x7c - 0x2c) == (a * 16) as u32
                    && word(&raw, 0x7c - 0x20) == (b * 16) as u32,
                "background loop indices differ: native {:?}, Rust {:?}",
                [word(&raw,0x7c-0x18),word(&raw,0x7c-0x28),word(&raw,0x7c-0x2c),word(&raw,0x7c-0x20)],
                [a as u32,b as u32,(a*16)as u32,(b*16)as u32]
            );
            ensure!(
                rect(&raw[0x7c - 0x40..]) == self.frame.rectangle()
                    && rect(&raw[0x7c - 0x60..]) == self.frame.damage(),
                "background pair rectangles differ"
            );
            if let Some(r) = self.frame.candidate() {
                ensure!(
                    rect(&raw[0x7c - 0x50..]) == r,
                    "background candidate differs"
                );
            }
        }
        if let Some(buffers) = s["buffers"].as_array() {
            ensure!(
                buffers.len() == 2,
                "native background pixel coverage differs"
            );
            for (r, p) in buffers.iter().zip(&self.pixels) {
                ensure!(
                    r["pointer"] == p.pointer
                        && r["pitch"] == p.pitch
                        && decode(&r["raw"])? == p.raw,
                    "background pixel bytes differ"
                );
            }
        }
        Ok(())
    }
}
