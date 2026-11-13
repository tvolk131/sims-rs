//! Original 57a54c fast return when the actual sound owner+14 byte is zero.
//! Enabled playback and debug tracing remain explicit unsupported services.
use super::*;
use sims_fixtures::readset::Readset;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disabled {
    application: u32,
    manager: u32,
    header: Vec<u8>,
    source_observation_sha256: String,
    source_code_sha256: String,
}
impl Disabled {
    pub fn import(state: &Value, m: &Readset<'_>, source: &Value) -> Result<Self> {
        ensure!(
            source["schema"] == 1
                && source["contract"] == "completed-disabled-sound-source-v1"
                && source["source_observation_sha256"]
                    == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299"
                && source["source_readset_sha256"] == m.source_sha256()
                && source["executes_target_code"] == false
                && source["physical_compressed_sha256"]
                    == state["physical_baseline"]["archive"]["compressed_sha256"]
                && source["physical_decoded_sha256"]
                    == state["physical_baseline"]["archive"]["decoded_sha256"],
            "disabled sound source epoch differs"
        );
        let code = source["code"]
            .as_array()
            .context("sound code receipts absent")?;
        ensure!(code.len() == 2, "sound code coverage differs");
        for (row, (address, size, digest)) in code.iter().zip([
            (
                0x591756_u32,
                13,
                "1d5296d6d18840ea4fce6932276df2b9a2c26e3b1eaa576eac14841c52844eb8",
            ),
            (
                0x57a54c,
                0x1ad,
                "b767c2dc2757aa19a35fe99557b8524b2a4d36e0868647e767555213f03a792b",
            ),
        ]) {
            let bytes = raw(&row["raw"])?;
            ensure!(
                row["address"] == address
                    && row["bytes"] == size
                    && bytes.len() == size
                    && row["sha256"] == digest
                    && hash(&bytes) == digest,
                "sound code witness differs"
            );
            crate::original_session::completed_skin::captured::overlap(m, address, &bytes)?;
        }
        let application = word(&m.read(0x6e48bc, 4)?, 0)?;
        let manager = word(
            &m.read(
                application
                    .checked_add(0x10)
                    .context("sound owner overflow")?,
                4,
            )?,
            0,
        )?;
        let header = m.read(manager, 24)?;
        ensure!(
            header[0x14] == 0,
            "enabled sound requires its actual mixer owner"
        );
        let rows = source["boundaries"]
            .as_array()
            .context("sound boundary inventory absent")?;
        ensure!(rows.len() == 33, "sound boundary inventory incomplete");
        for (index, row) in rows.iter().enumerate() {
            ensure!(
                row["index"] == index
                    && row["tick"] == 89073 + index
                    && row["application"] == application
                    && row["manager"] == manager
                    && raw(&row["header_raw"])? == header,
                "sound owner boundary identity differs"
            );
        }
        ensure!(
            rows[0]["readset_sha256"] == m.source_sha256(),
            "sound baseline receipt differs"
        );
        let result = Self {
            application,
            manager,
            header,
            source_observation_sha256: source["source_observation_sha256"].as_str().unwrap().into(),
            source_code_sha256: code[1]["sha256"].as_str().unwrap().into(),
        };
        result.validate_current(state, m)?;
        Ok(result)
    }
    pub fn validate_current(&self, state: &Value, m: &Readset<'_>) -> Result<()> {
        ensure!(
            state["readset"]["sha256"] == m.source_sha256()
                && state["tick"]
                    .as_u64()
                    .is_some_and(|n| (89073..=89105).contains(&n))
                && word(&m.read(0x6e48bc, 4)?, 0)? == self.application
                && word(
                    &m.read(
                        self.application
                            .checked_add(0x10)
                            .context("sound owner overflow")?,
                        4
                    )?,
                    0
                )? == self.manager
                && m.read(self.manager, 24)? == self.header
                && self.header.get(0x14) == Some(&0),
            "disabled sound owner changed or is outside qualified epoch"
        );
        Ok(())
    }
    pub fn property(
        &self,
        request: &properties::Request,
    ) -> Result<sims_sim::pose::refresh::Response> {
        ensure!(
            self.header.get(0x14) == Some(&0),
            "enabled sound is not a disabled-owner response"
        );
        match request {
            properties::Request::Sound { .. } => Ok(sims_sim::pose::refresh::Response::Done),
            _ => anyhow::bail!("property is outside original disabled-sound fast return"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "actual frozen B sound code and original 33 boundary owners"]
    fn actual_disabled_sound_owner_requires_source_and_preserves_state() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let folder=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-00");
        let encoded = std::fs::read(folder.join("observation.json.gz"))?;
        ensure!(
            hash(&encoded) == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299",
            "sound baseline changed"
        );
        let state: Value = serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw = std::fs::read(folder.join("acknowledged.mem"))?;
        let m = Readset::parse(&raw)?;
        let raw=std::fs::read(root.join("fixtures/private/goth-disabled-sound-20261030-first/original-disabled-sound-source-first.json"))?;
        ensure!(
            hash(&raw) == "6290847f79f2c79899a98b00583569412e287f772205b42fb64cdbf6e47af16f",
            "sound source changed"
        );
        let source: Value = serde_json::from_slice(&raw)?;
        let owner = Disabled::import(&state, &m, &source)?;
        let before = serde_json::to_vec(&owner)?;
        for object in [-2, 0, 18, 32767] {
            assert!(matches!(owner.property(&properties::Request::Sound{name:String::new(),object})?,sims_sim::pose::refresh::Response::Done));
        }
        assert_eq!(before, serde_json::to_vec(&owner)?);
        assert!(
            owner
                .property(&properties::Request::TraceSound {
                    text: "test".into()
                })
                .is_err()
        );
        let cold: Disabled = serde_json::from_slice(&before)?;
        assert_eq!(cold, owner);
        cold.validate_current(&state, &m)?;
        let mut wrong = source.clone();
        wrong["code"][1]["sha256"] = serde_json::json!("0".repeat(64));
        assert!(Disabled::import(&state, &m, &wrong).is_err());
        let mut wrong = source.clone();
        wrong["boundaries"][1]["manager"] = serde_json::json!(0);
        assert!(Disabled::import(&state, &m, &wrong).is_err());
        let mut enabled = owner.clone();
        enabled.header[0x14] = 1;
        assert!(
            enabled
                .property(&properties::Request::Sound {
                    name: "footstep_hard".into(),
                    object: 18
                })
                .is_err()
        );
        Ok(())
    }
}
