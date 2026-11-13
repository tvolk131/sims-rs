//! Original cached profiles and complete retained ordinary SavedGraphic values.
//! CPU material/update wrappers start unpublished at the completed caller;
//! native device allocation absence is not claimed by this comparison.
use super::*;
use sims_content::{Index, graphic_loading::SavedGraphic, graphic_profiles::Cached};
use sims_formats::{iff::Iff, strings::StringTable};
#[cfg(test)]
#[path = "profiles/tests.rs"]
mod tests;

fn exact<T: serde::de::DeserializeOwned + Serialize>(value: Value) -> Result<T> {
    let typed: T = serde_json::from_value(value.clone())?;
    ensure!(
        serde_json::to_value(&typed)? == value,
        "ordinary source projection omitted/defaulted fields"
    );
    Ok(typed)
}
fn completed_delivery(
    id: i16,
    tick: u64,
    value: &Value,
) -> Result<Option<sims_content::graphic_update::Update>> {
    let update: Option<sims_content::graphic_update::Update> =
        serde_json::from_value(value.clone())?;
    if let Some(update) = &update {
        update.validate(id)?;
        ensure!(
            tick == 88449
                && update.complete()
                && *value == json!({"object":id,"pass":"Done","child":null,"pending":null}),
            "ordinary replacement update is pending, has a callback/child, or differs from its completed owner"
        );
    }
    ensure!(
        serde_json::to_value(&update)? == *value,
        "ordinary delivery fields were discarded/defaulted"
    );
    Ok(update)
}
pub(super) fn evaluate(
    source: &Value,
    saved: &Saved,
    profile_raw: &[u8],
    proof: &Value,
    object_input: &Value,
    providers: &Value,
    index: &Index,
) -> Result<Vec<Value>> {
    ensure!(
        proof["schema"] == 1
            && proof["status"] == "native_ordinary_profiles_unchanged_qualified"
            && proof["before_tick"] == 88448
            && proof["after_tick"] == 88449
            && proof["profiles_sha256"] == sims_fixtures::digest(profile_raw)
            && proof["native_profile_metadata_and_strings_verified"] == true
            && proof["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && proof["coverage_payload_sha256"]
                == "e308ce89276b8d775a1820c160e495f7fde01491eea1a5b7621513ce8d18e393"
            && proof["target_executed"] == false,
        "ordinary current cache proof differs"
    );
    let fields = proof["fields"]
        .as_array()
        .context("ordinary cache proof fields absent")?;
    ensure!(
        fields.len() == 76
            && fields
                .iter()
                .all(|r| r["written_bytes"] == 0 && r["before_after_equal"] == true),
        "ordinary current native cache changed"
    );
    ensure!(
        object_input["tick"] == saved.simulation.state.vm.tick
            && object_input["provenance"]["replay_report_sha256"] == proof["source_report_sha256"],
        "ordinary object/source boundary differs"
    );
    let objects = sims_runtime::original_session::completed_object_storage::import(
        &serde_json::from_value::<
            Vec<sims_runtime::original_session::completed_object_storage::Input>,
        >(object_input["objects"].clone())?,
        "ordinary-native-current-object-observer",
    )?;
    let profiles: Value = serde_json::from_slice(profile_raw)?;
    let native_profiles = profiles["profiles"]
        .as_array()
        .context("native profiles absent")?;
    let actors = source["ordinary"]["actors"]
        .as_array()
        .context("native ordinary actors absent")?;
    ensure!(
        actors.len() == native_profiles.len()
            && actors.len() == saved.assets.ordinary.graphics.len(),
        "ordinary source/cache/current actor cohort differs"
    );
    let geometry = super::meshes::ordinary(source)?;
    let mut global = vec![];
    for g in [&source["geometry"], &source["ordinary"]["geometry"]] {
        for m in g["meshes"]
            .as_array()
            .context("shared native meshes absent")?
        {
            let p: u32 = serde_json::from_value(m["pointer"].clone())?;
            if !global.contains(&p) {
                global.push(p);
            }
        }
    }
    let mut caches = vec![];
    let mut graphics = BTreeMap::<i16, SavedGraphic>::new();
    let mut rows = vec![];
    let current_graphics = serde_json::to_value(&saved.assets.ordinary)?;
    for (i, a) in actors.iter().enumerate() {
        let id: i16 = serde_json::from_value(a["object"].clone())?;
        let p = &native_profiles[i];
        ensure!(p["object"] == id, "ordinary profile resource order differs");
        let ar = raw(&a["animator_raw"])?;
        let resource = raw(&a["resource_raw"])?;
        let def = raw(&a["definition_raw"])?;
        let definition = objects
            .get(&id)
            .and_then(|o| o.definition.as_ref())
            .context("original ordinary definition absent")?;
        ensure!(
            definition.source == format!("native-provider:{:08x}", word(&resource, 0)?)
                && definition.guid == word(&def, 0x1c)?
                && u32::from(definition.object) == word(&resource, 0xb4)? >> 16
                && p["cache"] == word(&resource, 0x88)?
                && p["provider"] == word(&resource, 0)?,
            "ordinary current cache/provider/definition alias differs"
        );
        let matches: Vec<_> = providers["candidate_module_bindings"]
            .as_array()
            .context("qualified ordinary providers absent")?
            .iter()
            .filter(|r| r["native_source"] == definition.source)
            .collect();
        ensure!(
            matches.len() == 1,
            "ordinary provider descriptor ambiguous/absent"
        );
        let provider = matches[0];
        let locator = provider["ordered_component_locators"][0]
            .as_str()
            .context("ordinary source locator absent")?;
        let file = index.resources[index.find(locator)?].read()?;
        ensure!(
            provider["primary_content_sha256"] == sims_fixtures::digest(&file),
            "ordinary current provider content version differs"
        );
        let iff = Iff::parse(file)?;
        let bank: u16 = serde_json::from_value(p["bank"].clone())?;
        let tables: Vec<_> = iff
            .resource_order(b"STR#")?
            .into_iter()
            .filter(|c| c.id == bank)
            .collect();
        ensure!(
            tables.len() == 1,
            "ordinary current native string bank ambiguous/absent"
        );
        let strings: Vec<String> = StringTable::parse(iff.data(tables[0]))?
            .entries
            .into_iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text)
            .collect();
        ensure!(
            serde_json::to_value(&strings)? == p["strings"],
            "original cached ordinary names differ from immutable provider"
        );
        let hash_bytes = completed_animations::bytes(&sims_fixtures::digest(&iff.bytes))?;
        let cached: Cached = exact(
            json!({"definition":definition,"state":{"kind":p["kind"],"id":bank,"file":1,"cached":1},
            "update":{"stage":"Done","constructed":null,"result":1,"pending":null},"profile":{"metadata":{"file":1,"id":bank,"tag":0x53545223u32,"flags":0,"language":0},"strings":strings},"source_hash":hash_bytes}),
        )?;
        caches.push(cached);
        let g = &geometry.characters[&id];
        let binding = super::meshes::ordinary_binding(a, g, id)?;
        let mut geom = g.geometry.clone();
        for binding in &mut geom.state.bindings {
            let p = geometry.native_meshes[binding.mesh];
            binding.mesh = global
                .iter()
                .position(|n| *n == p)
                .context("ordinary mesh outside actual shared pool")?;
        }
        let input: sims_content::graphic_retirement::observed::frame::Input =
            serde_json::from_value(a["input"].clone())?;
        // advance_update retains its Rust Update after Pass::Done. begin_update
        // accepts None or complete(); Done executes no operation. Preserve the
        // marker exactly, while rejecting any pending child or callback.
        let delivery = completed_delivery(
            id,
            saved.simulation.state.vm.tick,
            &current_graphics["graphics"][id.to_string()]["update"],
        )?;
        rows.push(json!({"object":id,"field":"ordinary_completed_Rust_delivery_storage","kind":"replacement_call_epoch","matched":true,
            "retained_value":delivery,"native_pending_work_absence_claimed":false,"source_mapping":"graphic_update::Update.complete/validate; Graphic.begin_update accepts a completed prior call"}));
        let expected: SavedGraphic = exact(
            json!({"marker_value":int(&ar,0x100)?,"owner":{"definition_flags":def[0xcc],"resource":i+1,"graphic":id,"ready":a["ready"]},"definition":definition,"profile":strings,
            "frame":{"clock_mode":input.fields.clock_mode,"now":input.fields.now,"visible":input.fields.visible,"blend":null,"placement":{"base":floats::<3>(&ar,0x80)?,"offset":floats::<3>(&ar,0x8c)?,"dirty":ar[0xd9]!=0,"explicit":ar[0x108]!=0},"matrix":{"values":floats::<16>(&ar,0x38)?,"kind":word(&ar,0x78)?}},
            "appearance":{"cached":int(&ar,0x9c)?,"dirty":ar[0xd9]!=0,"marker":ar[0xf8]!=0,"kind":word(&ar,0xfc)?,"ordinary_material":ar[0xf7]!=0,"tint":&ar[0xf4..0xf7]},
            "bound":super::meshes::bound(a,g,&source["fp_control"])?,"geometry":geom,"skeleton":"root","binding":binding,"materials":null,"update":delivery}),
        )?;
        same(
            &mut rows,
            id,
            "complete_ordinary_saved_graphic_source_state_and_unpublished_wrappers",
            &expected,
            &saved.assets.ordinary.graphics[&id],
        )?;
        graphics.insert(id, expected);
    }
    let value = serde_json::to_value(&saved.assets.ordinary)?;
    let current: Vec<Cached> = serde_json::from_value(value["profiles"].clone())?;
    same(
        &mut rows,
        0,
        "complete_original_ordinary_profile_cache_and_source_versions",
        &caches,
        &current,
    )?;
    // Rust delivery domains do not establish native retirement absence. The
    // source removal/caller requirement remains mandatory in the final gate.
    let expected: sims_content::graphic_loading::Saved = serde_json::from_value(
        json!({"profiles":caches,"jobs":{},"graphics":graphics,"retirements":{},"retained":{"graphics":{},"survivors":{},"refresh_context":null}}),
    )?;
    same(
        &mut rows,
        0,
        "complete_ordinary_registry_and_unpublished_delivery_domains",
        &expected,
        &saved.assets.ordinary,
    )?;
    Ok(rows)
}
