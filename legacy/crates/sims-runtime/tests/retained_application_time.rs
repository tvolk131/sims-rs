//! Differential original473509 arithmetic and exact typed time-owner resume.
use anyhow::{Result, ensure};
use serde::Deserialize;
use sims_vm::scheduler::clock::application;

#[derive(Deserialize)]
struct Case {
    label: String,
    before: [u32; 6],
    external_sample: u32,
    after: [u32; 6],
    callee_saved_and_return_stack_checked: bool,
}
#[derive(Deserialize)]
struct Oracle {
    original_function: u32,
    original_function_sha256: String,
    cases: Vec<Case>,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn application_clock_matches_original_instructions_and_retains_offset_on_resume() -> Result<()> {
    let oracle: Oracle = serde_json::from_str(include_str!(
        "../../sims-vm/tests/fixtures/native-application-time.json"
    ))?;
    ensure!(
        oracle.original_function == 0x473509
            && oracle.original_function_sha256
                == "d6da832824f53b8e72b904f58bd71bfde8047852c6a713bff46fc6f33bf90370"
            && oracle.cases.len() == 42,
        "original application clock fixture differs"
    );
    for case in oracle.cases {
        ensure!(
            case.callee_saved_and_return_stack_checked,
            "native ABI check absent"
        );
        let mut live = application::Fields::from_words(case.before);
        live.update(case.external_sample);
        ensure!(
            live.words() == case.after,
            "original clock mismatch: {}",
            case.label
        );
        let saved = postcard::to_allocvec(&live)?;
        let mut resumed: application::Fields = postcard::from_bytes(&saved)?;
        ensure!(
            resumed == live,
            "application clock discarded retained fields"
        );
        for sample in [
            case.external_sample,
            case.external_sample.wrapping_add(17),
            case.external_sample.wrapping_add(4000),
        ] {
            live.update(sample);
            resumed.update(sample);
            ensure!(
                resumed == live,
                "application clock resume changed offset/cadence"
            );
        }
    }
    Ok(())
}
