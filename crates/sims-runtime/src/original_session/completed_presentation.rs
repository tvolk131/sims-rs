//! Authenticate the completed cut's viewport, camera and property inputs.
//! These are logical service bindings, not a reconstructed native window tree.
pub mod sound;
use super::{character_properties, character_visibility};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sims_content::character_loading::{Presentation, RetirementEvent, RetirementReferences};
use sims_sim::{
    character::Resident,
    pose::{model::Matrix, properties, transform},
};
use sims_vm::{ObjectId, World};

fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn raw(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("presentation raw bytes absent")?;
    ensure!(
        s.is_ascii() && s.len() % 2 == 0 && s.len() <= 2 << 20,
        "presentation raw extent differs"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn word(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("presentation field outside raw owner")?
            .try_into()?,
    ))
}
fn read(v: &Value, address: u64, size: usize) -> Result<Vec<u8>> {
    let rows = v["reads"]
        .as_array()
        .context("presentation readset absent")?;
    let rows: Vec<_> = rows
        .iter()
        .filter(|r| {
            r["address"].as_u64() == Some(address) && r["bytes"].as_u64() == Some(size as u64)
        })
        .collect();
    ensure!(rows.len() == 1, "presentation read owner absent/ambiguous");
    let b = raw(&rows[0]["raw"])?;
    ensure!(
        b.len() == size && rows[0]["sha256"] == hash(&b),
        "presentation read hash differs"
    );
    Ok(b)
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bindings {
    pub viewport: [i32; 4],
    pub properties: character_properties::Source,
    pub visibility: character_visibility::Source,
    pub native_focus: u32,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub disabled_sound: Option<sound::Disabled>,
}

/// Whole supplied host ownership, still separate from complete logical Session
/// admission. Named fixture storage preserves exact simulation/asset values.
#[derive(Debug, Serialize, Deserialize)]
pub struct HostOwners {
    pub simulation: sims_sim::Snapshot,
    pub assets: sims_content::character_loading::live::Snapshot,
    pub presentation: Bindings,
    pub owned: sims_content::owned_resources::Neighborhood,
}
pub fn import(v: &Value, residents: &Value) -> Result<Bindings> {
    for source in [v, residents] {
        ensure!(
            source["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
                && source["provenance"]["observation_sha256"]
                    == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
            "presentation source epoch differs"
        );
    }
    ensure!(
        v["schema"] == 1
            && v["tick"] == 88448
            && v["native_parity_ticks"] == 0
            && v["executes_target_code"] == false,
        "presentation observation profile differs"
    );
    import_fields(v,residents)
}

/// Same source-code/getter/alias checks at a supplied current completed cut.
/// No window is constructed and current visible flags are never substituted
/// for the native model/camera visibility calculation.
pub fn import_captured(
    v:&Value,residents:&Value,tick:u64,memory:&sims_fixtures::readset::Readset<'_>,
) -> Result<Bindings> {
    ensure!(v["schema"]==1 && v["tick"]==tick && v["native_parity_ticks"]==0
        && v["executes_target_code"]==false,"current presentation profile differs");
    for r in v["reads"].as_array().context("current presentation reads absent")? {
        let b=raw(&r["raw"])?;
        ensure!(memory.read(r["address"].as_u64().context("presentation read address absent")?.try_into()?,b.len())?==b,
            "current presentation/readset differs");
    }
    import_fields(v,residents)
}
fn import_fields(v:&Value,residents:&Value)->Result<Bindings> {
    for (a, n, h) in [
        (
            0x415711,
            13,
            "1d798a068fb7fb7dab85563b70eca35c47caddfd8497518d73b29cb27e542c9b",
        ),
        (
            0x4aaf38,
            4,
            "9c8eb3adab3133652cdccbd58b010e59bcef792f39806367b4a21f5a9815fbf5",
        ),
        (
            0x585beb,
            144,
            "ddf01ba771d7ca4b0836258ca8b20155e564e074eae9b790b872b3fdac732543",
        ),
        (
            0x5a0a8a,
            478,
            "5c6c46e2cb7c0e32600bddce12b2d5e92feeb4454b554353cda4f9654a06f92f",
        ),
        (
            0x422632,
            51,
            "ec0b370dd4dfedc46161000af4d13861aed7e870c035835147b20ae4ee444883",
        ),
        (
            0x432595,
            32,
            "8d719be67b0dfa62aea7f568d669e709a8413e6e22b78ccb532a6d1d4c1f3f0f",
        ),
    ] {
        ensure!(
            hash(&read(v, a, n)?) == h,
            "presentation source code differs"
        );
    }
    let owner = word(&read(v, 0x6e48bc, 4)?, 0)?;
    ensure!(
        v["owner"].as_u64() == Some(owner as u64),
        "presentation process owner differs"
    );
    let owner_raw = read(v, owner as u64, 0x60)?;
    ensure!(
        owner_raw == raw(&v["owner_raw"])?,
        "presentation process storage differs"
    );
    let viewer = word(&owner_raw, 4)?;
    let simulator = word(&owner_raw, 0x1c)?;
    ensure!(
        v["viewer"].as_u64() == Some(viewer as u64),
        "presentation viewer differs"
    );
    let viewer_raw = read(v, viewer as u64, 0x180)?;
    ensure!(
        viewer_raw == raw(&v["viewer_raw"])?,
        "presentation viewer storage differs"
    );
    let vt = read(v, word(&viewer_raw, 0)? as u64, 0x28)?;
    ensure!(
        vt == raw(&v["viewer_vtable_raw"])? && word(&vt, 0x10)? == 0x4aaf38,
        "presentation camera getter differs"
    );
    let device = word(&viewer_raw, 0x54)?;
    ensure!(
        v["device"].as_u64() == Some(device as u64),
        "presentation render device differs"
    );
    let camera = read(v, device as u64 + 0xec, 68)?;
    ensure!(
        camera == raw(&v["camera_raw"])?,
        "presentation camera bytes differ"
    );
    let values: [f32; 16] = std::array::from_fn(|i| f32::from_bits(word(&camera, i * 4).unwrap()));
    ensure!(
        values.iter().all(|f| f.is_finite()),
        "presentation camera is nonfinite"
    );
    let viewport: [i32; 4] =
        std::array::from_fn(|i| word(&viewer_raw, 0x28 + i * 4).unwrap() as i32);
    ensure!(
        v["viewport"] == serde_json::to_value(viewport)?
            && viewport[2] > viewport[0]
            && viewport[3] > viewport[1],
        "presentation viewport differs"
    );
    let enabled = read(v, 0x6d0458, 1)?;
    ensure!(
        enabled == raw(&v["visibility_enabled_raw"])?,
        "presentation visibility enable differs"
    );
    let lot = read(v, simulator as u64 + 0x24, 2)?;
    ensure!(
        lot == raw(&v["lot_raw"])? && v["lot_raw"] == residents["resident_input"]["lot_raw"],
        "presentation lot input differs"
    );
    ensure!(
        raw(&v["debug_sound_raw"])? == [owner_raw[0x5b]]
            && residents["resident_input"]["debug_raw"].as_u64() == Some(owner_raw[0x5b] as u64),
        "presentation debug input differs"
    );
    let focus = read(v, 0x6f6bac, 4)?;
    ensure!(
        focus == raw(&v["global_focus_raw"])?,
        "presentation focus input differs"
    );
    Ok(Bindings {
        disabled_sound: None,
        viewport,
        properties: character_properties::Source {
            lot_kind: i16::from_le_bytes(lot.as_slice().try_into()?),
            debug_sound: owner_raw[0x5b] != 0,
        },
        visibility: character_visibility::Source {
            camera: Matrix {
                values,
                kind: word(&camera, 64)?,
            },
            enabled: enabled[0] != 0,
        },
        native_focus: word(&focus, 0)?,
    })
}

/// A bounded diagnostic host uses only the actually bound services. Unbound
/// frontend operations fail; there are no fabricated pending UI continuations.
impl Presentation for Bindings {
    fn context_in_world(
        &mut self,
        id: ObjectId,
        r: &Resident,
        w: &World,
    ) -> Result<properties::Context> {
        self.properties.context(id, r, w)
    }
    fn context(&mut self, _: ObjectId, _: &transform::Placement) -> Result<properties::Context> {
        anyhow::bail!("captured context requires world")
    }
    fn viewport(&mut self) -> Result<[i32; 4]> {
        Ok(self.viewport)
    }
    fn visibility_for_model(
        &mut self,
        _: ObjectId,
        r: &Resident,
        m: &sims_content::character_model::State,
        v: [i32; 4],
    ) -> Result<bool> {
        self.visibility.visible(r, m, v)
    }
    fn visibility(&mut self, _: ObjectId, _: [i32; 4]) -> Result<bool> {
        anyhow::bail!("captured visibility requires model")
    }
    fn retirement_references(&mut self, _: ObjectId) -> Result<RetirementReferences> {
        anyhow::bail!("captured retirement frontend unbound")
    }
    fn retire(&mut self, _: ObjectId, _: RetirementEvent) -> Result<()> {
        anyhow::bail!("captured retirement frontend unbound")
    }
    fn appearance(
        &mut self,
        _: ObjectId,
        _: &sims_content::character_appearance::Request,
    ) -> Result<()> {
        anyhow::bail!("captured appearance frontend unbound")
    }
    fn saved_texture(&mut self, _: ObjectId) -> Result<()> {
        anyhow::bail!("captured texture frontend unbound")
    }
    fn invalidate(&mut self, _: ObjectId) -> Result<()> {
        anyhow::bail!("captured invalidation frontend unbound")
    }
    fn property(
        &mut self,
        _: ObjectId,
        request: &properties::Request,
    ) -> Result<sims_sim::pose::refresh::Response> {
        self.disabled_sound.as_ref().context("captured property frontend unbound")?.property(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};
    fn captured() -> Result<Option<(Value, Value)>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join(
            "fixtures/private/goth-presentation-owners-20261028-first/presentation-second.json",
        );
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some((
            serde_json::from_slice(&fs::read(path)?)?,
            serde_json::from_slice(&fs::read(root.join(
                "fixtures/private/goth-character-residents-20261027-third/residents-input.json",
            ))?)?,
        )))
    }
    #[test]
    fn captured_getter_identity_and_viewport_cannot_be_substituted() -> Result<()> {
        let Some((v, residents)) = captured()? else {
            return Ok(());
        };
        let b = import(&v, &residents)?;
        ensure!(
            b.viewport == [0, 0, 800, 600]
                && b.properties.lot_kind == 5
                && b.visibility.enabled
                && b.native_focus == 0,
            "captured presentation values differ"
        );
        let mut wrong = v.clone();
        wrong["device"] = serde_json::json!(v["device"].as_u64().unwrap() + 4);
        ensure!(
            import(&wrong, &residents).is_err(),
            "substituted camera device accepted"
        );
        wrong = v.clone();
        wrong["viewport"][2] = serde_json::json!(799);
        ensure!(
            import(&wrong, &residents).is_err(),
            "substituted viewport accepted"
        );
        wrong = v.clone();
        wrong["provenance"]["image_sha256"] = serde_json::json!("00".repeat(32));
        ensure!(
            import(&wrong, &residents).is_err(),
            "mixed source cut accepted"
        );
        Ok(())
    }
    #[test]
    fn captured_services_roundtrip_and_unbound_paths_fail() -> Result<()> {
        let Some((v, residents)) = captured()? else {
            return Ok(());
        };
        let mut b = import(&v, &residents)?;
        let encoded = serde_json::to_vec(&b)?;
        let cold: Bindings = serde_json::from_slice(&encoded)?;
        ensure!(b == cold, "bound service state roundtrip differs");
        ensure!(
            b.context(
                21,
                &transform::Placement {
                    base: [0.; 3],
                    previous: [0.; 3],
                    offset: [0.; 3],
                    dirty: false
                }
            )
            .is_err(),
            "native context used a renderer-only fallback"
        );
        ensure!(
            b.visibility(21, b.viewport).is_err(),
            "native visibility used a boolean fallback"
        );
        let mut old = super::super::View::default();
        let encoded = serde_json::to_value(&old)?;
        ensure!(
            encoded.get("character_properties").is_none()
                && encoded.get("character_visibility").is_none(),
            "historical view acquired native inputs"
        );
        old.character_properties = Some(cold.properties);
        old.character_visibility = Some(cold.visibility);
        let coldview: super::super::View = serde_json::from_value(serde_json::to_value(&old)?)?;
        ensure!(old == coldview, "view lost explicit native inputs");
        Ok(())
    }
}
