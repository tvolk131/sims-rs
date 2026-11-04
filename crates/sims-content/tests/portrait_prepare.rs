use anyhow::{Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "requires private original temporary-person capture and assets"]
fn original_portrait_forces_selection_before_collecting_head_models() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-temporary-person-placed-session.json"),
    )?)?;
    let id = 196;
    for replace in [false, true] {
        let mut session = Session::restore(&index, saved.clone())?;
        let assets = session.host.characters.get_mut(&id).unwrap();
        let person = session.simulation.vm.world.objects[&id]
            .person
            .as_ref()
            .unwrap();
        assets.appearance_owner.selection.cached = [8, 21, 22, 23].map(|n| i32::from(person[&n]));
        assets.appearance_owner.selection.dirty = false;
        session
            .simulation
            .characters
            .get_mut(&id)
            .unwrap()
            .character
            .placement
            .dirty = false;
        let head = assets.avatar.records.head;
        let old_binding = assets.avatar.records.entries[head].binding;
        if replace {
            // The source name is still resolvable (asset lookup ignores case),
            // but native descriptor prefix comparison is case-sensitive. This
            // forces real detach/rebind while keeping the original head asset.
            let record = &mut assets.avatar.records.entries[head];
            record.name = record.name.to_ascii_uppercase();
        }
        let before = session.save();
        let mut restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&before)?)?,
        )?;
        // An unsupported tint must fail even with matching selector caches:
        // this proves force=1 did not silently take the cached early return.
        session
            .host
            .characters
            .get_mut(&id)
            .unwrap()
            .appearance_owner
            .selection
            .tint = [1, 2, 3];
        let failed = session.save();
        ensure!(
            session
                .host
                .prepare_portrait(&mut session.simulation, &session.program, id)
                .is_err()
        );
        ensure!(
            session.simulation == failed.simulation.state
                && session.host.snapshot() == failed.assets
        );
        session
            .host
            .characters
            .get_mut(&id)
            .unwrap()
            .appearance_owner
            .selection
            .tint = [0; 3];
        let selected =
            session
                .host
                .prepare_portrait(&mut session.simulation, &session.program, id)?;
        let after_restore =
            restored
                .host
                .prepare_portrait(&mut restored.simulation, &restored.program, id)?;
        ensure!(selected == after_restore && !selected.is_empty());
        ensure!(
            session.simulation == restored.simulation
                && session.host.snapshot() == restored.host.snapshot()
        );
        let assets = &session.host.characters[&id];
        ensure!(assets.appearance_owner.selection.dirty);
        ensure!(session.simulation.characters[&id].character.placement.dirty);
        ensure!((assets.avatar.records.entries[head].binding != old_binding) == replace);
        ensure!(
            assets
                .avatar
                .avatar
                .parts
                .iter()
                .any(|p| p.attachment != "HEAD")
        );
        for &(binding, part) in &selected {
            ensure!(assets.avatar.avatar.parts[part].attachment == "HEAD");
            ensure!(assets.geometry.vertices.contains_key(&binding));
        }
        // The preparation changes appearance/cache state, not VM execution,
        // interactions, physical placement, person fields or simulator RNG.
        ensure!(session.simulation.vm.tick == before.simulation.state.vm.tick);
        ensure!(session.simulation.vm.threads == before.simulation.state.vm.threads);
        ensure!(session.simulation.vm.world.objects == before.simulation.state.vm.world.objects);
        let after = session.save();
        let post = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&after)?)?,
        )?;
        ensure!(
            post.simulation == session.simulation
                && post.host.snapshot() == session.host.snapshot()
        );
        ensure!(
            post.host.characters[&id]
                .geometry
                .portrait_bindings(&post.host.characters[&id].avatar)?
                == selected
        );
        // A subsequent forced pass with unchanged descriptors does not replace
        // geometry or allocate a fresh binding just because force is true.
        ensure!(
            session
                .host
                .prepare_portrait(&mut session.simulation, &session.program, id)?
                == selected
        );
        ensure!(
            session.simulation == after.simulation.state && session.host.snapshot() == after.assets
        );
        if replace {
            std::fs::write(
                root.join("cache/original-portrait-prepared-session.json"),
                serde_json::to_vec(&after)?,
            )?;
        }
        println!(
            "replacement={replace}: {} exact HEAD bindings, forced selection, atomic error and before/after restore verified",
            selected.len()
        );
    }
    Ok(())
}
