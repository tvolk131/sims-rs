//! Actual higher caller suspended inside46d9d6 at the two completed cuts.
//! This qualifies source inputs; it does not manufacture a runtime call/frame.
use super::completed_lot::Memory;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub mod captured;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub in_update: u8,
    pub suppressed: u8,
    /// Exact byte+60. Its broader meaning is not inferred from the callback.
    pub post_clock_flag: u8,
}
pub struct Imported {
    pub fields: Fields,
    pub source: Value,
}
impl Imported {
    /// Retain the proven waiting caller around its actual clock child. This
    /// does not begin an application update or dispatch its return callback.
    pub fn runtime_parent(
        &self,
        child: &super::clock::State,
        tick: u64,
    ) -> Result<super::application_parent::State> {
        ensure!(
            self.source["tick"] == tick,
            "application/clock completed cuts differ"
        );
        super::application_parent::State::from_completed_inputs(self.fields.clone(), child, tick)
    }
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("application source bytes absent")?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("application source pointer absent")?
        .try_into()?)
}
pub fn import(v: &Value) -> Result<Imported> {
    ensure!(
        v["schema"] == 1
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["executes_target_code"] == false
            && v["native_parity_ticks"] == 0
            && v["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "application source cut/epoch differs"
    );
    let m = Memory::new(v)?;
    let code = m.read(0x4d1901, 566)?;
    ensure!(
        sims_fixtures::digest(&code)
            == "13f790293f95a2e0c75f1e862be46135d7da46efe66406ff2fbb12ab3299cd97"
            && v["code_sha256"] == sims_fixtures::digest(&code),
        "application instruction profile differs"
    );
    let ebp = 8519172;
    let stack = m.read(ebp - 336, 360)?;
    ensure!(
        v["parent"]["ebp"] == ebp
            && v["parent"]["return_pc"] == 0x4d1ab8u32
            && v["parent"]["outer_return"] == 0x4f7563u32
            && bytes(&v["parent"]["stack"])? == stack
            && word(&stack, 0) == 0x4d1ab8
            && word(&stack, 340) == 0x4f7563,
        "application actual suspended return/stack differs"
    );
    let receiver = word(&stack, 336 - 0x10);
    let raw = m.read(receiver, 0x1c0)?;
    ensure!(
        v["receiver"] == receiver
            && bytes(&v["raw"])? == raw
            && word(&raw, 0) == 0x668d74
            && v["vtable"] == 0x668d74u32,
        "application active receiver differs"
    );
    let callback = word(&m.read(0x668d74 + 0x168, 4)?, 0);
    let callback_code = m.read(callback, 5)?;
    ensure!(
        callback == 0x5de715
            && v["after_clock_callback"] == callback
            && sims_fixtures::digest(&callback_code) == "46eada20450a6b6f8e09daac29adb4f006003d8603766bb4989a349a40e5c67d"
            && bytes(&v["callback_code"])? == callback_code,
        "application post-clock virtual differs"
    );
    let global = word(&m.read(0x6f2a58, 4)?, 0);
    let global_raw = m.read(global, 0xb4)?;
    ensure!(
        global != receiver
            && number(&v["global_owner"]["pointer"])? == global
            && bytes(&v["global_owner"]["raw"])? == global_raw
            && v["global_owner"]["control_state"] == word(&global_raw, 0xb0)
            && v["word_b0"] == word(&raw, 0xb0),
        "application distinct interface/receiver aliases differ"
    );
    let fields = Fields {
        in_update: raw[0x118],
        suppressed: raw[0x119],
        post_clock_flag: raw[0x60],
    };
    ensure!(
        fields
            == (Fields {
                in_update: 1,
                suppressed: 0,
                post_clock_flag: 1
            })
            && v["update_in_progress"] == fields.in_update
            && v["update_suppressed"] == fields.suppressed
            && v["byte_60"] == fields.post_clock_flag,
        "application captured gate/flag fields differ"
    );
    Ok(Imported {
        fields,
        source: json!({"schema":1,"tick":v["tick"],"receiver":receiver,"separate_global_interface":global,
        "CPState":word(&global_raw,0xb0),"native_parent":"4d1901 awaiting46d9d6 at4d1ab8",
        "actual_stack_and_physical_callback_qualified":true,"runtime_frame_imported":false,
        "complete_native_Session_admitted":false,"native_parity_ticks":0}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private actual original application-parent source"]
    fn actual_parent_rejects_merged_receivers_false_frames_and_flags() -> Result<()> {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/private/goth-supplemental-owners-20261029-first/application-parent-fourth");
        let before: Value = serde_json::from_slice(&std::fs::read(root.join("before.json"))?)?;
        let after: Value = serde_json::from_slice(&std::fs::read(root.join("after.json"))?)?;
        ensure!(
            import(&before)?.fields == import(&after)?.fields,
            "actual parent flags changed across cut"
        );
        for i in 0..7 {
            let mut bad = before.clone();
            match i {
                0 => bad["receiver"] = bad["global_owner"]["pointer"].clone(),
                1 => bad["global_owner"]["pointer"] = bad["receiver"].clone(),
                2 => bad["update_in_progress"] = 0.into(),
                3 => bad["parent"]["return_pc"] = 0.into(),
                4 => bad["callback_code"] = "c3".into(),
                5 => bad["tick"] = 999.into(),
                _ => bad["provenance"]["replay_report_sha256"] = "wrong".into(),
            }
            ensure!(
                import(&bad).is_err(),
                "false application parent {i} admitted"
            );
        }
        Ok(())
    }
}
