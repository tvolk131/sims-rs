//! CPState/transport from the separately captured 89073..89105 source epoch.
//! The original fixed 88448 qualification remains unchanged.
use super::*;
use sims_fixtures::readset::Readset;

pub fn import(state: &Value, memory: &Readset<'_>) -> Result<Imported> {
    ensure!(
        state["schema"] == 1
            && state["tick"]
                .as_u64()
                .is_some_and(|t| (89073..=89105).contains(&t))
            && state["readset"]["sha256"] == memory.source_sha256()
            && state["code"]["reference_image_sha256"]
                == "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321",
        "captured control source epoch differs"
    );
    let input = &state["domains"]["controls"];
    ensure!(
        input["schema"] == 1
            && input["tick"] == state["tick"]
            && input["executes_target_code"] == false,
        "captured control domain differs"
    );
    crate::original_session::completed_skeletons::captured::reads(input, memory)?;
    let controls = decode(input, &Memory::new(input)?)?;
    let simulator = number(&input["simulator"])?;
    ensure!(
        state["tick"] == word(&memory.read(simulator + 0x98, 4)?, 0),
        "captured control tick differs from simulator storage"
    );
    Ok(controls)
}
