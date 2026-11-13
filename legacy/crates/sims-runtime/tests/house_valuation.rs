use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session};
use sims_sim::house_valuation as value;
use std::path::Path;

#[test]
#[ignore = "private independently managed three-home checkpoints; no native game execution"]
fn three_homes_value_and_refresh_without_running_ticks_and_restore_independently() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("valuation test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_LIVE_VALUATION_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let baseline = repo.join("cache/managed-independent-20261022-third");
    let index = Index::scan(&baseline.join("immutable-assets"))?;
    let mut cases = vec![];
    let mut architecture_cases = vec![];
    let mut reports = vec![];
    for name in ["newbie", "goth", "mashuga"] {
        let saved: Saved = serde_json::from_slice(&std::fs::read(
            baseline.join(format!("{name}-tick16.json")),
        )?)?;
        let mut s = Session::restore(&index, saved)?;
        let before = s.simulation.clone();
        let family = before.vm.world.person.as_ref().unwrap().controlled_family;
        let metadata = before.vm.world.person.as_ref().unwrap().family_metadata[&family].clone();
        let result = s
            .update_house_valuation()
            .with_context(|| format!("{name} live valuation"))?;
        let size = result.globals[0] as usize;
        let inputs = s
            .simulation
            .original_valuation_inputs(&s.program, size)
            .map_err(anyhow::Error::msg)?;
        let objects = value::objects(&inputs).map_err(anyhow::Error::msg)?;
        cases.push(serde_json::json!({"household":name,"objects":inputs,"rust":objects}));
        let lot = s
            .simulation
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap();
        let mut floors = lot
            .floors
            .iter()
            .map(|f| f.source.clone())
            .collect::<Vec<_>>();
        for (floor, flags) in floors
            .iter_mut()
            .zip(&s.simulation.native_lot.as_ref().unwrap().placement.flags)
        {
            floor.flags.clone_from(flags);
        }
        let prices = &s
            .host
            .presentation
            .valuation_catalog
            .as_ref()
            .unwrap()
            .prices;
        let architecture =
            value::architecture(size, lot.size, &floors, &lot.pool, prices, |_, _| {})
                .map_err(anyhow::Error::msg)?;
        architecture_cases.push(serde_json::json!({"household":name,"size":size,"stride":lot.size,"floors":floors,
            "pool":lot.pool,"style_prices":prices.styles,"pool_price":prices.pool,"floor_prices":prices.floors,
            "wall_prices":prices.walls,"rust":architecture}));
        let mut expected = before;
        expected.household.as_mut().unwrap().valuation = Some(value::Retained {
            objects_value: result.objects_value,
            architecture_value: result.architecture_value,
        });
        expected
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .family_metadata
            .get_mut(&family)
            .unwrap()
            .flags = result.family_flags;
        for (key, n) in [23, 22, 37].into_iter().zip(result.globals) {
            expected.vm.world.globals.insert(key, n);
        }
        ensure!(
            expected == s.simulation,
            "{name}: valuation changed simulation beyond native publication stores"
        );
        ensure!(
            s.simulation
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .family_metadata[&family]
                .dirty
                == metadata.dirty,
            "valuation dirtied family"
        );
        let mut refreshed = s.simulation.clone();
        let active: Vec<_> = refreshed
            .room_environment
            .as_ref()
            .unwrap()
            .manager
            .rooms
            .iter()
            .filter_map(|(&id, room)| room.active.then_some(id))
            .collect();
        let refresh = refreshed.vm.world.refresh.as_mut().unwrap();
        for id in active {
            refresh.rooms.get_mut(&id).unwrap().dirty = true;
        }
        refresh.dirty_categories = 3;
        for c in refreshed.route_control.values_mut() {
            if let Some(original) = c.original.as_mut() {
                original.cache_valid = false;
            }
        }
        for cache in refreshed.portal_cache.values_mut() {
            for cost in cache.costs.values_mut() {
                *cost = 10001.;
            }
        }
        ensure!(
            s.refresh_original_house()? == result,
            "{name}: repeated valuation changed"
        );
        ensure!(
            s.simulation == refreshed,
            "{name}: full refresh changed unexpected state"
        );
        let managed = s.save_managed()?;
        let before = serde_json::to_vec(&s.save())?;
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&managed)?)?,
        )?;
        ensure!(
            before == serde_json::to_vec(&restored.save())?,
            "{name}: valuation restore differs"
        );
        let catalog = s.host.presentation.valuation_catalog.as_ref().unwrap();
        let build = catalog
            .sources
            .keys()
            .find(|source| source.to_ascii_lowercase().contains("build.iff"))
            .context("valuation Build.iff dependency absent")?;
        let mut changed_assets = index.clone();
        let resource = changed_assets.find(build)?;
        let mut bytes = changed_assets.resources[resource].read()?;
        bytes[0] ^= 1;
        changed_assets.resources[resource].owned = Some(std::sync::Arc::from(bytes));
        ensure!(
            Session::restore(&changed_assets, managed.clone()).is_err(),
            "{name}: changed valuation catalog accepted"
        );
        let mut lost_owner = managed.clone();
        lost_owner.view.valuation_catalog = None;
        ensure!(
            Session::restore(&index, lost_owner).is_err(),
            "{name}: missing valuation catalog accepted"
        );
        std::fs::write(
            output.join(format!("{name}-valued.json")),
            serde_json::to_vec(&managed)?,
        )?;
        reports.push(serde_json::json!({"household":name,"tick":s.simulation.vm.tick,"publication":result,
            "only_expected_simulation_stores":true,"family_dirty_unchanged":true,"independent_full_session_restore_equal":true,
            "changed_price_asset_rejected":true,"missing_price_owner_rejected":true,
            "full_refresh_only_expected_stores":true,
            "native_parity_ticks_added":0,"continuation_ticks_added":0}));
    }
    std::fs::write(
        output.join("objects-inputs.json"),
        serde_json::to_vec(&serde_json::json!({"cases":cases}))?,
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    std::fs::write(
        output.join("architecture-inputs.json"),
        serde_json::to_vec(&serde_json::json!({"cases":architecture_cases}))?,
    )?;
    Ok(())
}
