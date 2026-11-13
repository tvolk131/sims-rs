//! Source-owned pointer-vector backing for the retained action caller. The
//! caller owns its logical lists/header words; storage keeps unused allocation
//! bytes across erase/rebuild. Native growth and active animation stay pending.
use super::*;
use sims_vm::scheduler::clock::draw::actions::tail as native;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    windows: Vec<u8>,
    scratch: Vec<u8>,
    windows_written: u8,
    scratch_written: u8,
}
pub(super) fn import(source: &[u8], headers: &[[u32; 3]]) -> Result<Owner> {
    ensure!(
        sims_fixtures::digest(source)
            == "a8af1f2e8e7010d140c7a8d4570f100b8dacc5a53a6fb603c93fe1d9cf0d7524",
        "action tail source differs"
    );
    let value: serde_json::Value = serde_json::from_slice(source)?;
    let m = Memory::new(&value)?;
    ensure!(headers.len() == 4, "action vector headers absent");
    let owner = Owner {
        windows: m.read(headers[0][0], (headers[0][2] - headers[0][0]) as usize)?,
        scratch: m.read(headers[3][0], (headers[3][2] - headers[3][0]) as usize)?,
        windows_written: 1,
        scratch_written: 1,
    };
    owner.validate_sources(headers)?;
    Ok(owner)
}
impl Owner {
    fn validate_sources(&self, headers: &[[u32; 3]]) -> Result<()> {
        ensure!(
            headers.len() == 4
                && headers[0][0] == 0x0edc8d90
                && headers[0][2] == 0x0edc8d98
                && headers[3][0] == 0x0ea8a5a8
                && headers[3][2] == 0x0ea8a5b0,
            "action pointer-vector allocation changed without growth"
        );
        for (h, b, written) in [
            (&headers[0], &self.windows, self.windows_written),
            (&headers[3], &self.scratch, self.scratch_written),
        ] {
            native::validate(h, b).map_err(anyhow::Error::msg)?;
            ensure!(
                (1..=2).contains(&written) && (h[1] - h[0]) / 4 <= written as u32,
                "action vector write extent differs"
            );
            if written == 1 {
                ensure!(
                    word(b, 4) == 0x0b4c64c0,
                    "unused action vector allocation bytes changed"
                );
            }
        }
        Ok(())
    }
    pub(super) fn validate(&self, headers: &[[u32; 3]], state: &actions::State) -> Result<()> {
        self.validate_sources(headers)?;
        for (h, b, values) in [
            (&headers[0], &self.windows, &state.windows),
            (&headers[3], &self.scratch, &state.scratch),
        ] {
            ensure!(
                (h[1] - h[0]) as usize == values.len() * 4
                    && values.iter().enumerate().all(|(i, &p)| word(b, i * 4) == p),
                "action vector backing/header/list disagree"
            );
        }
        Ok(())
    }
    pub(super) fn sync_caller_erase(
        &self,
        headers: &mut [[u32; 3]],
        state: &actions::State,
    ) -> Result<()> {
        self.validate_sources(headers)?;
        for (index, b, values) in [
            (0, &self.windows, &state.windows),
            (3, &self.scratch, &state.scratch),
        ] {
            let h = &mut headers[index];
            let old_len = (h[1] - h[0]) as usize / 4;
            if old_len != values.len() {
                ensure!(
                    values.is_empty(),
                    "caller changed a pointer-vector outside whole-range erase"
                );
                native::clear(h, b).map_err(anyhow::Error::msg)?;
            }
        }
        self.validate(headers, state)
    }
    pub(super) fn service(
        &mut self,
        request: &actions::Request,
        headers: &mut [[u32; 3]],
        state: &mut actions::State,
    ) -> Result<bool> {
        self.validate(headers, state)?;
        match *request {
            actions::Request::Push { vector, window } => {
                let (index, b, values) = match vector {
                    actions::Vector::Windows => (0, &mut self.windows, &mut state.windows),
                    actions::Vector::Scratch => (3, &mut self.scratch, &mut state.scratch),
                    actions::Vector::Records => return Ok(false),
                };
                if let native::Append::Growth { .. } =
                    native::append(&mut headers[index], b, window).map_err(anyhow::Error::msg)?
                {
                    return Ok(false);
                }
                values.push(window);
                let count = values.len() as u8;
                if index == 0 {
                    self.windows_written = self.windows_written.max(count);
                } else {
                    self.scratch_written = self.scratch_written.max(count);
                }
                self.validate(headers, state)?;
                Ok(true)
            }
            actions::Request::Advance { window } => native::idle_advance(
                state
                    .headers
                    .get(&window)
                    .ok_or_else(|| anyhow::anyhow!("action animation window absent"))?,
            )
            .map_err(anyhow::Error::msg),
            _ => Ok(false),
        }
    }
    #[cfg(test)]
    pub(super) fn compare_native(
        &self,
        raw: &[u8],
        headers: &[[u32; 3]],
        state: &actions::State,
    ) -> Result<()> {
        ensure!(
            sims_fixtures::digest(raw)
                == "0609f8a4c29d96b05808fe9f50f193c788fe153edec995ac6db0b9bcbd31c3da",
            "native Goth action tail result differs"
        );
        let result: serde_json::Value = serde_json::from_slice(raw)?;
        let decode = |value: &serde_json::Value| -> Result<Vec<u8>> {
            let s = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("native tail bytes absent"))?;
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
                .collect()
        };
        for (label, index, backing) in [
            ("windows", 0, Some(&self.windows)),
            ("scratch", 3, Some(&self.scratch)),
            ("records", 2, None),
        ] {
            let v = result["vectors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["label"] == label)
                .unwrap();
            let header: [u32; 3] = serde_json::from_value(v["header"].clone())?;
            ensure!(header == headers[index], "original vector metadata differs");
            let mut expected = decode(&v["raw"])?;
            let actual = if let Some(backing) = backing {
                ensure!(
                    *backing == decode(&v["allocation"])?,
                    "original pointer-vector allocation differs"
                );
                if index == 0 {
                    state
                        .windows
                        .iter()
                        .copied()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>()
                } else {
                    state
                        .scratch
                        .iter()
                        .copied()
                        .flat_map(u32::to_le_bytes)
                        .collect()
                }
            } else {
                ensure!(
                    expected.len() == 28 && state.records.len() == 1,
                    "original tail record extent differs"
                );
                let actual = state.records.concat();
                expected[4..8].copy_from_slice(&actual[4..8]);
                let base = word(&expected, 16);
                let managed = word(&actual, 16);
                for offset in [16, 20, 24] {
                    let mapped = word(&expected, offset)
                        .wrapping_sub(base)
                        .wrapping_add(managed);
                    expected[offset..offset + 4].copy_from_slice(&mapped.to_le_bytes());
                }
                actual
            };
            ensure!(actual == expected, "original tail vector payload differs");
        }
        let mut window = decode(&result["window_after"])?;
        window[0xdc..0xe0].copy_from_slice(&state.headers[&0x0b7c7cb0][0xdc..0xe0]);
        ensure!(
            state.headers[&0x0b7c7cb0] == window
                && state.headers[&0x0b4c64c0] == decode(&result["retired_after"])?,
            "original action-window exit differs"
        );
        Ok(())
    }
}
