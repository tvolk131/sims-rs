//! Ordered original OS replies, separate from expected game-state observations.
//! This stream qualifies one observed World GetClientRect call at C8. A later
//! call or different boundary needs its own input; no rectangle is fabricated.
use super::word;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ClientReply {
    pub tick: u64,
    pub window: u32,
    pub result: u32,
    pub rectangle: [u32; 4],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    source: String,
    calls: Vec<ClientReply>,
    next: usize,
}
const SOURCE: &str = "626be002d91ae886e5ff601dc918855ff0b3be5fce6a5469ce55615e7f0b0a14";
impl Inputs {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "original OS window inputs differ"
        );
        let mut decoded = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut decoded)?;
        let rows: Vec<serde_json::Value> = decoded
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let calls: Vec<_> = rows
            .iter()
            .filter(|r| {
                r["kind"] == "same_stop_outer_readback" && r["named_api"]["name"] == "GetClientRect"
            })
            .filter_map(|r| {
                let stack =
                    super::super::completed_animations::bytes(r["named_api"]["stack"].as_str()?)
                        .ok()?;
                (word(&stack, 0) == 0x4a89ad).then_some((r, stack))
            })
            .collect();
        ensure!(calls.len() == 1, "World OS reply input is absent/ambiguous");
        let (entry, stack) = &calls[0];
        let output = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == 0x4a89ad)
            .context("World OS return input absent")?;
        ensure!(
            entry["tick"] == 89081
                && output["tick"] == 89081
                && entry["named_api"]["address"] == 0x688dfd60
                && entry["named_api"]["export_source_sha256"]
                    == "89ea1db9b798b1c08c7c5012b8b149a35392befa6e89ce11bbca4b635de1cde4",
            "World OS reply identity differs"
        );
        let raw = super::super::completed_animations::bytes(
            output["world_stack"]["raw"]
                .as_str()
                .context("OS rectangle output absent")?,
        )?;
        let bp = output["registers"]["ebp"]
            .as_u64()
            .context("OS output frame absent")? as usize;
        let sp = output["world_stack"]["address"]
            .as_u64()
            .context("OS output stack absent")? as usize;
        let at = bp
            .checked_sub(0x48 + sp)
            .context("OS output slot outside frame")?;
        let raw = raw
            .get(at..at + 16)
            .context("OS rectangle output outside stack")?;
        ensure!(
            word(stack, 8) == bp as u32 - 0x48,
            "World OS output pointer alias differs"
        );
        let call = ClientReply {
            tick: 89081,
            window: word(stack, 4),
            result: output["registers"]["eax"]
                .as_u64()
                .context("OS raw result absent")?
                .try_into()?,
            rectangle: std::array::from_fn(|i| word(raw, i * 4)),
        };
        let result = Self {
            source: SOURCE.into(),
            calls: vec![call],
            next: 0,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn consumed(&self) -> usize {
        self.next
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && self.calls
                    == [ClientReply {
                        tick: 89081,
                        window: 0xa006a,
                        result: 1,
                        rectangle: [0, 0, 800, 600]
                    }]
                && self.next <= self.calls.len(),
            "OS window input stream differs"
        );
        Ok(())
    }
    pub(super) fn peek(&self, tick: u64, window: u32) -> Result<Option<ClientReply>> {
        self.validate()?;
        let Some(reply) = self.calls.get(self.next).copied() else {
            return Ok(None);
        };
        ensure!(
            reply.tick == tick && reply.window == window,
            "OS window input does not match pending call"
        );
        Ok(Some(reply))
    }
    pub(super) fn consume(&mut self, reply: ClientReply) -> Result<()> {
        ensure!(
            self.calls.get(self.next) == Some(&reply),
            "OS window input is not next"
        );
        self.next += 1;
        self.validate()
    }
}
