//! Actual CPState/interface inputs; no GUI, constructor or publication runs.
use super::{completed_lot::Memory, retained_frontend::Fields};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_sim::lot_presenter::camera_command::Ui;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub camera: Ui,
    pub context: Fields,
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("frontend bytes absent")?)
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("frontend pointer absent")?.try_into()?)
}
fn word(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
pub fn import(v: &Value, tools: &Value, application: &Value) -> Result<Imported> {
    ensure!(
        v["schema"] == 2
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["tick"] == tools["tick"]
            && v["tick"] == application["tick"]
            && v["executes_target_code"] == false,
        "frontend boundary/profile differs"
    );
    ensure!(
        v["source_observation_sha256"]
            == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
            && v["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && v["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f",
        "frontend original source differs"
    );
    let m = Memory::new(v)?;
    ensure!(
        sims_fixtures::digest(&m.read(0x4ce44f, 0x398)?)
            == "f73ef287fb4ffa80802c4cc929e6c17674150e9dfd1667e0d316c7ed8c15b7bd"
            && v["frontend"]["publication_code_sha256"]
                == "f73ef287fb4ffa80802c4cc929e6c17674150e9dfd1667e0d316c7ed8c15b7bd",
        "native publication instruction mapping differs"
    );
    let owner = word(&m.read(0x6f2a58, 4)?, 0);
    ensure!(
        owner == number(&v["frontend"]["pointer"])?
            && owner == number(&application["global_owner"]["pointer"])?
            && owner != number(&application["receiver"])?,
        "frontend application aliases differ"
    );
    let interface = m.read(owner, 0x100)?;
    ensure!(word(&interface,0x90)==0,
            "non-null native menu descriptor requires its retained callback owner");
    let parent_global = bytes(&application["global_owner"]["raw"])?;
    ensure!(
        interface == bytes(&v["frontend"]["raw"])?
            && parent_global.len() <= interface.len()
            && interface[..parent_global.len()] == parent_global,
        "frontend interface body differs"
    );
    let pointer = word(&interface, 0xb0);
    ensure!(
        pointer == number(&v["frontend"]["CPState"]["pointer"])?
            && pointer == number(&tools["CPState"]["pointer"])?,
        "frontend CPState alias differs"
    );
    let raw = m.read(pointer, 0x220)?;
    ensure!(
        raw == bytes(&tools["CPState"]["raw"])?
            && raw == bytes(&v["frontend"]["CPState"]["raw"])?
            && word(&raw, 0) == 0x668d34,
        "frontend concrete CPState differs"
    );
    Ok(Imported {
        camera: Ui {
            flags: raw[0xd],
            zoom: word(&raw, 0x34),
            rotation: word(&raw, 0x38),
            floor: word(&raw, 0x3c) as i32,
        },
        context: Fields {
            panel_owner: (word(&raw, 0x54) != 0).then_some(u64::from(word(&raw, 0x54))),
            menu_present: word(&interface, 0x90) != 0,
            menu_active: interface[0xf5],
            closing: interface[0xf8],
            walls: word(&raw, 0x48) as i32,
            tool_gate: word(&raw, 0x60),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private actual CPState/interface observations"]
    fn actual_publication_inputs_preserve_idle_context_without_a_frame() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |name: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(root.join(name))?)?)
        };
        let b = "fixtures/private/goth-input-ownership-20261029-second/tool-payloads-third";
        let old = "fixtures/private/goth-supplemental-owners-20261029-first";
        let v = read(&format!("{b}/before.json"))?;
        let tools = read(&format!("{old}/input-tools-second/before.json"))?;
        let app = read(&format!("{old}/application-parent-fourth/before.json"))?;
        let actual = import(&v, &tools, &app)?;
        ensure!(
            actual.camera
                == Ui {
                    flags: 0,
                    zoom: 1,
                    rotation: 0,
                    floor: 1
                }
                && actual.context
                    == Fields {
                        panel_owner: Some(0x09e27078),
                        menu_present: false,
                        menu_active: 0,
                        closing: 0,
                        walls: 4,
                        tool_gate: 0
                    },
            "actual persistent frontend values differ"
        );
        let after = read(&format!("{b}/after.json"))?;
        ensure!(
            actual
                == import(
                    &after,
                    &read(&format!("{old}/input-tools-second/after.json"))?,
                    &read(&format!("{old}/application-parent-fourth/after.json"))?
                )?,
            "native persistent frontend changed across recorded tick"
        );
        ensure!(
            serde_json::from_value::<Fields>(serde_json::to_value(&actual.context)?)?
                == actual.context,
            "frontend context codec differs"
        );
        for fault in 0..4 {
            let mut bad = v.clone();
            match fault {
                0 => bad["frontend"]["CPState"]["pointer"] = 0.into(),
                1 => bad["frontend"]["raw"] = "00".into(),
                2 => bad["source_report_sha256"] = "wrong".into(),
                _ => bad["tick"] = 88449.into(),
            }
            ensure!(
                import(&bad, &tools, &app).is_err(),
                "altered native frontend alias/epoch accepted"
            );
        }
        Ok(())
    }
}
