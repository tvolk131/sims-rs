//! Explicit field coverage for the replacement wrapper at this qualified
//! original boundary. Native equality still requires the separate source
//! comparisons named in the obligations. No empty Rust call proves a native
//! queue empty; the original suspended VM state stays in its retained owners.
use super::{Saved, View, completed_presentation, completed_viewer};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
pub mod assets;
pub mod effects;
pub mod lifetimes;
pub mod options;
pub mod residents;
pub mod saved;
pub mod session;
pub mod simulation;
pub mod source_assets;
pub mod sources;
#[cfg(test)]
mod tests;
pub mod thread_owners;
pub mod vm;
pub mod world;

fn fact(name: &str, kind: &str, matched: bool, rows: &mut Vec<Value>) {
    rows.push(json!({"field":name,"kind":kind,"matched":matched}));
}
fn obligation(name: &str, comparison: &str, rows: &mut Vec<Value>) {
    rows.push(json!({"field":name,"kind":"native_comparison_obligation","comparison":comparison}));
}

pub(super) fn same<T: Serialize>(name: &str, native: &T, rust: &T, rows: &mut Vec<Value>) -> Result<()> {
    let a = postcard::to_allocvec(native)?;
    let b = postcard::to_allocvec(rust)?;
    rows.push(
        json!({"field":name,"kind":"native_or_source_derived_equality","matched":a==b,
        "native_sha256":sims_fixtures::digest(&a),"rust_sha256":sims_fixtures::digest(&b)}),
    );
    Ok(())
}
pub fn view(saved: &Saved, source: &Value, closure: &Value) -> Result<Value> {
    ensure!(
        saved.view.neighborhood_replacement.is_none(),
        "replacement neighborhood input is disqualified from native-equality evidence"
    );
    ensure!(
        [88448, 88449].contains(&saved.simulation.state.vm.tick)
            && source["proof"]["before_tick"] == 88448
            && source["proof"]["after_tick"] == 88449
            && source["proof"]["captured_payloads_equal"] == true,
        "completed View epoch differs"
    );
    let viewer = completed_viewer::import(&source["viewer"], &source["lot"])?;
    let inputs = completed_presentation::import(&source["presentation"], &source["resident"])?;
    // Validates the actual caller proof; these idle values are replacement
    // wrappers around source-owned state, not native constructor results.
    let (expected_presenter, expected_notifications, expected_portrait) =
        viewer.runtime_owners(closure)?;
    let View {
        resource_aliases,
        character_properties,
        character_visibility,
        person_draw,
        neighborhood_input,
        neighborhood_replacement: _,
        valuation_catalog,
        person_selection_override,
        current_tool,
        retained_tool_inputs,
        publication,
        tool_selection,
        move_tool,
        move_button,
        object_focus,
        purchase_cursor,
        object_budget,
        picker_target,
        picker_activation_word,
        ground_input,
        object_input,
        live_menu,
        ui_portrait_tick,
        control_panel,
        interface_update,
        family_window,
        transport,
        clock,
        application_parent,
        ui_wait,
        camera_ui,
        frontend_context,
        camera_command,
        loading,
        postload,
        awaiting_house_open,
        selected,
        viewport,
        camera,
        visible,
        presenter,
        notifications,
        property_refresh,
    } = &saved.view;
    let mut rows = vec![];
    macro_rules! absent { ($($field:ident),+ $(,)?)=>{$(
        rows.push(json!({"field":stringify!($field),"kind":"unpublished_replacement_call",
            "matched":$field.is_none(),"native_empty_queue_claimed":false}));
    )+}; }
    absent!(
        person_draw,
        neighborhood_input,
        publication,
        tool_selection,
        move_tool,
        move_button,
        purchase_cursor,
        object_budget,
        ground_input,
        object_input,
        live_menu,
        ui_portrait_tick,
        control_panel,
        interface_update,
        family_window,
        ui_wait,
        camera_command,
        loading,
        postload,
        property_refresh
    );
    // These presence checks are obligations, not evidence of equality. The
    // final gate must require their named native comparers as well.
    macro_rules! bound { ($($field:ident),+ $(,)?)=>{$(
        rows.push(json!({"field":stringify!($field),"kind":"required_native_owner",
            "matched":$field.is_some(),"native_equality_required_separately":true}));
    )+}; }
    bound!(
        resource_aliases,
        valuation_catalog,
        current_tool,
        retained_tool_inputs,
        picker_activation_word,
        transport,
        clock,
        application_parent,
        camera_ui,
        frontend_context
    );
    rows.push(
        json!({"field":"retained_tool_inputs/stock","kind":"required_native_owner",
        "matched":retained_tool_inputs.as_ref().and_then(|s|s.stock()).is_some(),
        "native_equality_required_separately":true}),
    );
    rows.push(
        json!({"field":"picker_target","kind":"native_comparison_obligation",
        "comparison":"tools-{before,after}/picker_target","value":picker_target}),
    );
    same(
        "character_properties",
        &Some(inputs.properties),
        character_properties,
        &mut rows,
    )?;
    same(
        "character_visibility",
        &Some(inputs.visibility),
        character_visibility,
        &mut rows,
    )?;
    same(
        "person_selection_override",
        &Some(viewer.person_selection_override),
        person_selection_override,
        &mut rows,
    )?;
    ensure!(
        viewer.object_focus == 0 && inputs.native_focus == 0,
        "non-NULL focus requires original identity mapping"
    );
    same("object_focus", &None::<u32>, object_focus, &mut rows)?;
    same("viewport", &inputs.viewport, viewport, &mut rows)?;
    same("camera", &Some(expected_portrait), camera, &mut rows)?;
    same("presenter", &Some(expected_presenter), presenter, &mut rows)?;
    same(
        "notifications",
        &Some(expected_notifications),
        notifications,
        &mut rows,
    )?;
    let actual_selection = saved
        .simulation
        .state
        .native_lot
        .as_ref()
        .context("native selection owner absent")?
        .selected_person;
    same(
        "selected/retained_native_lot_alias",
        &actual_selection,
        selected,
        &mut rows,
    )?;
    rows.push(
        json!({"field":"visible","kind":"unused_legacy_override","matched":visible.is_none(),
        "native_visibility_owner":"character_visibility"}),
    );
    rows.push(
        json!({"field":"awaiting_house_open","kind":"unpublished_replacement_call",
        "matched":!*awaiting_house_open,"native_empty_queue_claimed":false}),
    );
    //42 direct fields plus one nested mandatory stock owner. No source file
    //or serde omission is allowed to choose a smaller field inventory.
    ensure!(rows.len() == 43, "View field coverage inventory changed");
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparisons":rows,
        "difference_count":differences,"native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All42 View fields explicitly classified, mandatory source bindings checked, complete fresh presenter/notification wrappers and independent portrait input compared. Named native comparison obligations remain mandatory; this is not full Session admission."}),
    )
}
