//! Named external point replies, separate from expected game-state masks.
use super::*;
use std::io::Read;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Call {
    tick: u64,
    name: String,
    window: Option<u32>,
    input: Option<[i32; 2]>,
    result: u32,
    point: [i32; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    source: String,
    calls: Vec<Call>,
    next: usize,
}
fn number(v: &serde_json::Value) -> Result<u32> {
    Ok(v.as_u64().context("cursor trace word absent")?.try_into()?)
}
fn bytes(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("cursor trace bytes absent")?,
    )
}
fn point(b: &[u8]) -> Result<[i32; 2]> {
    ensure!(b.len() == 8, "cursor POINT extent differs");
    Ok([word(b, 0) as i32, word(b, 4) as i32])
}
impl Inputs {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        ensure!(
            source == "5e3a09d2f0040060fc598991af91022829561bf7d8bb751ce31ec861c3b448bc",
            "named cursor witness differs"
        );
        let mut raw = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut raw)?;
        let rows = raw
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<serde_json::Value>, _>>()?;
        let mut calls = vec![];
        for (index, row) in rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.get("cursor_reply").is_some())
        {
            let ret = number(&row["pc"])?;
            let reply = &row["cursor_reply"];
            let name = reply["name"].as_str().context("cursor reply name absent")?;
            ensure!(
                (name == "GetCursorPos" && ret == 0x4abcc5)
                    || (name == "ScreenToClient" && ret == 0x4abcdd),
                "cursor return caller differs"
            );
            let entry = rows[..index]
                .iter()
                .rev()
                .find(|r| r["named_api"]["name"] == name)
                .context("named cursor entry absent")?;
            let api = &entry["named_api"];
            let stack = bytes(&api["stack"])?;
            ensure!(
                word(&stack, 0) == ret
                    && api["export_source_sha256"]
                        == "08f7b223b366316570f7767958d2112a54615447ea4c5868e653bffad5e15a86",
                "cursor named entry differs"
            );
            let is_client = name == "ScreenToClient";
            ensure!(
                number(&entry["pc"])? == if is_client { 0x688dff20 } else { 0x688b6f70 },
                "cursor export address differs"
            );
            ensure!(
                word(&stack, if is_client { 8 } else { 4 }) == number(&reply["address"])?
                    && number(&row["registers"]["esp"])?
                        == number(&entry["registers"]["esp"])?.wrapping_add(if is_client {
                            12
                        } else {
                            8
                        }),
                "cursor output pointer/cleanup differs"
            );
            ensure!(
                row["tick"] == 89081 && entry["tick"] == row["tick"],
                "cursor call epoch differs"
            );
            calls.push(Call {
                tick: 89081,
                name: name.into(),
                window: is_client.then(|| word(&stack, 4)),
                input: if is_client {
                    Some(point(&bytes(&api["rectangle_raw"])?)?)
                } else {
                    None
                },
                result: number(&reply["result"])?,
                point: point(&bytes(&reply["raw"])?)?,
            });
        }
        ensure!(
            calls.len() == 4
                && calls.iter().enumerate().all(|(i, c)| c.name
                    == if i % 2 == 0 {
                        "GetCursorPos"
                    } else {
                        "ScreenToClient"
                    }),
            "cursor World call ordering differs"
        );
        let result = Self {
            source,
            calls,
            next: 0,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == "5e3a09d2f0040060fc598991af91022829561bf7d8bb751ce31ec861c3b448bc"
                && self.calls.len() == 4
                && self.next <= self.calls.len(),
            "cursor input owner differs"
        );
        Ok(())
    }
    pub fn consumed(&self) -> usize {
        self.next
    }
    pub fn service(&mut self, view: &mut View, tick: u64) -> Result<bool> {
        self.validate()?;
        view.validate()?;
        let Some(request) = view.pending() else {
            return Ok(false);
        };
        let Some(call) = self.calls.get(self.next) else {
            return Ok(false);
        };
        ensure!(tick == call.tick, "cursor reply tick differs");
        match request {
            Request::GetCursorPos => ensure!(
                call.name == "GetCursorPos" && call.window.is_none() && call.input.is_none(),
                "cursor input kind differs"
            ),
            Request::ScreenToClient { window, point } => ensure!(
                call.name == "ScreenToClient"
                    && call.window == Some(window)
                    && call.input == Some(point),
                "client cursor receiver/input differs"
            ),
            Request::Project => return Ok(false),
        }
        view.reply_point(request, call.result, call.point)?;
        self.next += 1;
        Ok(true)
    }
}
