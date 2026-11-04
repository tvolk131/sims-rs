//! Read-only CPState/transport inputs at the captured completed boundary.
//! The adapter call epoch is new; no native constructor or setter executes.
use super::completed_lot::Memory;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_vm::{
    interface::{buy, publication, speed, state},
    scheduler::{clock::Timer, transport::Timing},
};
pub mod captured;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Imported {
    pub controls: state::State,
    pub timing: Timing,
}
impl Imported {
    pub fn into_transport(self) -> super::transport::State {
        super::transport::State::from_completed_inputs(self.controls, self.timing)
    }
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("control pointer absent")?.try_into()?)
}
fn word(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn hex(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("control bytes absent")?)
}
pub fn import(v: &Value) -> Result<Imported> {
    ensure!(
        v["schema"] == 1
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["executes_target_code"] == false
            && v["native_parity_ticks"] == 0,
        "control boundary profile differs"
    );
    ensure!(
        v["provenance"]["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "control source epoch differs"
    );
    decode(v, &Memory::new(v)?)
}

fn decode(v: &Value, m: &Memory) -> Result<Imported> {
    for (a, n, h) in [
        (
            0x4ce0b9,
            0x290,
            "e88d3905b4649409a6216d42c5d00cad0f855b59b5bc25b6dfecc3e301976372",
        ),
        (
            0x4ce390,
            0xbf,
            "88a0114a4167aba89cf545a6c42b8afa7bedb25902d4dd05ae66ad433076729a",
        ),
        (
            0x4cee4a,
            0x3f,
            "009985d7855d16d05dc71468029cf9911de8cd13cf10161f3b05eada969c424f",
        ),
        (
            0x4cee89,
            0xb0,
            "02b8a00f449694142bf2888d0680dbc273011bb225a0365bd7effc09118ab11e",
        ),
        (
            0x46d813,
            0x60,
            "62a6c8f084985e0f214862c1f034a64804ff64a63433eb78d4c084488585cb3b",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(a, n)?) == h,
            "control instruction mapping differs"
        );
    }
    let application = word(&m.read(0x6f2a58, 4)?, 0);
    let pointer = word(&m.read(application + 0xb0, 4)?, 0);
    ensure!(
        number(&v["application"])? == application && number(&v["pointer"])? == pointer,
        "control application/CPState aliases differ"
    );
    let b = m.read(pointer, 0x220)?;
    ensure!(
        b == hex(&v["raw"])? && word(&b, 0) == 0x668d34,
        "control concrete owner differs"
    );
    let owner = word(&m.read(0x6e48bc, 4)?, 0);
    let simulator = word(&m.read(owner + 0x1c, 4)?, 0);
    ensure!(
        number(&v["simulator"])? == simulator,
        "control simulator alias differs"
    );
    let timing = m.read(simulator + 0xb8, 8)?;
    ensure!(
        timing == hex(&v["timing_raw"])?,
        "control timing storage differs"
    );
    let timer = |off| Timer {
        elapsed: word(&b, off),
        interval: word(&b, off + 4),
        last: word(&b, off + 8),
        limit: word(&b, off + 12),
    };
    let controls = state::State {
        mode: word(&b, 8) as i32,
        paused: b[0x40],
        speed: word(&b, 0x44) as i32,
        selected_person_index: Some(word(&b, 0x1ec) as i32),
        speed_timers: Some(speed::Timers {
            first: timer(0x200),
            second: timer(0x210),
        }),
        publication: Some(publication::Fields {
            after_dirty: word(&b, 0x10),
            last: word(&b, 0x14),
        }),
        build_tool: word(&b, 0x68) as i32,
        buy_tool: word(&b, 0x9c) as i32,
        buy: buy::Fields {
            dirty: word(&b, 0xc),
            catalog: word(&b, 0xa0) as i32,
            subcatalog: word(&b, 0xa4) as i32,
            indices: std::array::from_fn(|i| {
                std::array::from_fn(|j| word(&b, 0xa8 + 8 * i + 4 * j) as i32)
            }),
            cells: (0..64).map(|i| word(&b, 0xec + 4 * i)).collect(),
            selected_table: word(&b, 0xe0),
            selected_object: word(&b, 0xe4),
            selection_aux: word(&b, 0xe8),
        },
    };
    controls.validate().map_err(anyhow::Error::msg)?;
    Ok(Imported {
        controls,
        timing: Timing {
            last: word(&timing, 0),
            elapsed: word(&timing, 4),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private source-only completed CPState observation"]
    fn actual_controls_require_exact_source_owner_and_timers() -> Result<()> {
        let v:Value=serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/private/goth-dependent-graphic-20261029-first/controls-before-first.json"))?)?;
        let got = import(&v)?;
        assert_eq!(got.controls.mode, 2);
        assert_eq!(got.controls.speed, 760);
        assert_eq!(got.controls.selected_person_index, Some(326));
        assert_eq!(got.controls.buy.dirty, 0);
        for key in ["pointer", "simulator"] {
            let mut x = v.clone();
            x[key] = serde_json::json!(0);
            assert!(import(&x).is_err());
        }
        let mut x = v.clone();
        let mut timing = hex(&x["timing_raw"])?;
        timing[0] ^= 1;
        x["timing_raw"] = serde_json::json!(
            timing
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(import(&x).is_err());
        let mut x = v.clone();
        x["raw"] = serde_json::json!("00".repeat(0x220));
        assert!(import(&x).is_err());
        let mut x = v;
        x["provenance"]["image_sha256"] = serde_json::json!("wrong");
        assert!(import(&x).is_err());
        Ok(())
    }
}
