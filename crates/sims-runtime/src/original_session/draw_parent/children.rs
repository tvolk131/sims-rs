//! Source-bound nonrecording view and idle unload owners. Unread header bytes
//! remain retained; these profiles do not authorize recording/loading work.
use super::{Memory, Result, draw, ensure, word};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owners {
    view: Vec<u8>,
    unload: Vec<u8>,
}
pub(super) fn import(source: &[u8]) -> Result<Owners> {
    ensure!(
        sims_fixtures::digest(source)
            == "a6a57cd017bb3ee3bada146f972d01967f1d61cf6f3a26ed4ed4731067758145",
        "draw service source differs"
    );
    let value: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        value["schema"] == 1 && value["tick"] == 89073 && value["executes_target_code"] == false,
        "draw service source epoch differs"
    );
    let memory = Memory::new(&value)?;
    for (address, size, hash) in [
        (
            0x4d4ffd,
            374,
            "f119336c64f46882f858ad1552bbd7280253e9fff4216325dc99ccd73765b831",
        ),
        (
            0x46fb64,
            1,
            "ae3f4619b0413d70d3004b9131c3752153074e45725be13b9a148978895e359e",
        ),
        (
            0x4ffbb3,
            23,
            "6ec124b2c3cb985f25346759e355133f9505cf6dddcd0b9014148db1fe4d59df",
        ),
        (
            0x4ffd1a,
            3,
            "b32aef6a0de5404219950914e20fd4f32743cd4fbdf36d5909fb15c796071999",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&memory.read(address, size)?) == hash,
            "draw service code differs"
        );
    }
    let view = memory.read(0x0ac04dd8, 0x100)?;
    let table = word(&view, 0);
    ensure!(
        word(&memory.read(table + 0x30, 4)?, 0) == 0x4d4ffd
            && word(&memory.read(table + 0x34, 4)?, 0) == 0x46fb64,
        "draw service virtual targets differ"
    );
    let app = memory.read(0x0315a250, 0x11c0)?;
    ensure!(
        word(&app, 0x174) == 0x0ac04dd8 && word(&app, 0x11ac) == 0x0a9ab808,
        "draw service application aliases differ"
    );
    let owners = Owners {
        view,
        unload: memory.read(0x0a9ab808, 0x140)?,
    };
    owners.validate(false)?;
    Ok(owners)
}
impl Owners {
    #[cfg(test)]
    pub(super) fn headers(&self) -> (&[u8], &[u8]) {
        (&self.view, &self.unload)
    }
    pub(super) fn validate(&self, past_begin: bool) -> Result<()> {
        ensure!(
            self.view.len() == 0x100 && self.unload.len() == 0x140,
            "draw service owner extent differs"
        );
        ensure!(
            word(&self.view, 4) == 0 && word(&self.unload, 0x100) == 0,
            "recording or active unload branch is not admitted"
        );
        ensure!(
            word(&self.view, 0x38) == u32::from(!past_begin),
            "view frame-word disagrees with saved Draw cursor"
        );
        let mut original = self.view.clone();
        original[0x38..0x3c].copy_from_slice(&1u32.to_le_bytes());
        ensure!(
            sims_fixtures::digest(&original)
                == "6552086da21f00e1786f9f8d9f4c6ea27fadba3a407c4847c898349bbc777a1d"
                && sims_fixtures::digest(&self.unload)
                    == "d5533a26f47e12032b555b808c082d04a39942e8dd9a3136f21b316c57c7f356",
            "unaccessed draw service owner bytes changed"
        );
        Ok(())
    }
    pub(super) fn supports(&self, request: draw::Request) -> bool {
        matches!(
            request,
            draw::Request::ViewBegin { owner: 0x0ac04dd8 }
                | draw::Request::ViewEnd { owner: 0x0ac04dd8 }
                | draw::Request::Unload { owner: 0x0a9ab808 }
        )
    }
    pub(super) fn apply(&mut self, request: draw::Request) -> Result<()> {
        ensure!(self.supports(request), "draw service does not own child");
        if matches!(request, draw::Request::ViewBegin { .. }) {
            let mut view = draw::view::View {
                mode: word(&self.view, 4),
                frame_word: word(&self.view, 0x38),
            };
            ensure!(
                view.begin() == draw::view::Next::Return,
                "recording child needs its own caller"
            );
            self.view[0x38..0x3c].copy_from_slice(&view.frame_word.to_le_bytes());
        }
        if matches!(request, draw::Request::Unload { .. }) {
            ensure!(
                draw::view::unload_returns(word(&self.unload, 0x100)),
                "active unload needs its own caller"
            );
        }
        Ok(())
    }
    pub(super) fn view_frame_word(&self) -> u32 {
        word(&self.view, 0x38)
    }
}
