use anyhow::{Result, ensure};
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    character_model,
};
use sims_runtime::original_session::{Options, Session};
use sims_sim::{
    character::Resident,
    lot_route::path::Policy,
    pose::{
        model::Bounds,
        refresh::{Request, Response},
    },
};

// Frozen ModelBounds arm of the previous copy/with_bound/acknowledge/commit
// transaction. Keep the redundant writes: this is the behavioral control.
fn previous_service(
    geometry: &mut CharacterGeometry,
    model: &mut character_model::State,
    resident: &mut Resident,
    pool: &mut MeshPool,
) -> Result<Option<Vec<usize>>> {
    ensure!(resident.character.pending_request() == Some(&Request::ModelBounds));
    let next = geometry.clone();
    let next_pool = pool.clone();
    let mut next_model = model.clone();
    let mut next_resident = resident.clone();
    let notifications = next_model.with_bound(&mut next_resident, |bound| {
        bound.render_bounds = bound.local_bounds;
        bound.model.bounds_flags &= !1;
        Ok(vec![])
    })?;
    next_resident
        .character
        .acknowledge(Response::Done)
        .map_err(anyhow::Error::msg)?;
    *geometry = next;
    *pool = next_pool;
    *model = next_model;
    *resident = next_resident;
    Ok(Some(notifications))
}

type Owners = (
    Resident,
    character_model::State,
    CharacterGeometry,
    MeshPool,
);

fn round_trip(owners: &Owners) -> Result<Owners> {
    let restored = serde_json::from_slice(&serde_json::to_vec(owners)?)?;
    ensure!(
        *owners == restored,
        "geometry service snapshot changed state"
    );
    Ok(restored)
}

#[test]
#[ignore = "requires private original houses and assets"]
fn original_model_bounds_matches_copy_transaction_and_resumes() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        // Reach a real ordinary-update service boundary without fabricating
        // private continuation fields or advancing the VM past the service.
        let mut boundary = None;
        for _ in 0..1000 {
            session.host.update_contexts(&mut session.simulation)?;
            let result = session
                .simulation
                .run_current_tick(&session.program, 100_000);
            boundary = session
                .simulation
                .pending_character_services()
                .into_iter()
                .find_map(|(id, request)| (request == Request::ModelBounds).then_some(id));
            if boundary.is_some() {
                break;
            }
            let count = session
                .host
                .service(&mut session.simulation, &mut session.program)?;
            if count == 0 {
                let step = result.map_err(anyhow::Error::msg)?;
                anyhow::bail!("expected ModelBounds in first tick, got {:?}", step.stop);
            }
        }
        let id = boundary.ok_or_else(|| anyhow::anyhow!("ModelBounds not reached: {source}"))?;
        let assets = &session.host.characters[&id];
        let initial = (
            session.simulation.characters[&id].clone(),
            assets.model.clone(),
            assets.geometry.clone(),
            session.host.mesh_pool.clone(),
        );
        ensure!(!initial.2.vertices.is_empty() && !initial.3.meshes.is_empty());

        // Preserve unrelated flag bits, matrix tags, cache contents and both
        // empty/nonempty bounds. Also restore at each side of the service.
        for flags in [0, 1, 7, 0x8000_0001] {
            for kind in [0, 1, 2, 99] {
                let mut actual = initial.clone();
                actual.0.character.model.bounds_flags = flags;
                actual.0.character.model.bounds_transform.kind = kind;
                actual.1.local_bounds = if flags == 0 {
                    Bounds::EMPTY
                } else {
                    Bounds {
                        minimum: [-9., -3., 2.],
                        maximum: [1., 7., 11.],
                    }
                };
                actual.1.render_bounds = Bounds {
                    minimum: [1.; 3],
                    maximum: [4.; 3],
                };
                actual = round_trip(&actual)?;
                let mut expected = actual.clone();
                let old_result = previous_service(
                    &mut expected.2,
                    &mut expected.1,
                    &mut expected.0,
                    &mut expected.3,
                )?;
                let result = actual.2.service_resident(
                    &mut actual.1,
                    &mut actual.0,
                    &assets.avatar,
                    &mut actual.3,
                )?;
                ensure!(
                    result == old_result && actual == expected,
                    "ModelBounds differed: {source}/{flags}/{kind}"
                );
                actual = round_trip(&actual)?;
                // An already acknowledged request must remain unadvanced.
                ensure!(
                    actual
                        .2
                        .service_resident(
                            &mut actual.1,
                            &mut actual.0,
                            &assets.avatar,
                            &mut actual.3
                        )?
                        .is_none()
                );
                ensure!(actual == expected);
            }
        }

        // The old publication guard uses floating-point equality: identical
        // NaN bit patterns still fail. All four owners must remain untouched.
        for nan in [false, true] {
            let mut actual = initial.clone();
            if nan {
                actual.0.character.matrix[0] = f32::NAN;
                actual.0.character.model.bounds_transform.values[0] = f32::NAN;
            } else {
                actual.0.character.matrix[12] += 1.;
            }
            let mut expected = actual.clone();
            let before = serde_json::to_vec(&actual)?;
            let matrix_bits = actual.0.character.matrix.map(f32::to_bits);
            let transform_bits = actual
                .0
                .character
                .model
                .bounds_transform
                .values
                .map(f32::to_bits);
            let old_error = previous_service(
                &mut expected.2,
                &mut expected.1,
                &mut expected.0,
                &mut expected.3,
            )
            .unwrap_err();
            let error = actual
                .2
                .service_resident(&mut actual.1, &mut actual.0, &assets.avatar, &mut actual.3)
                .unwrap_err();
            ensure!(
                error.to_string() == old_error.to_string()
                    && error.to_string() == "resident model transform is not published"
            );
            ensure!(
                serde_json::to_vec(&actual)? == before && serde_json::to_vec(&expected)? == before
            );
            ensure!(actual.0.character.matrix.map(f32::to_bits) == matrix_bits);
            ensure!(
                actual
                    .0
                    .character
                    .model
                    .bounds_transform
                    .values
                    .map(f32::to_bits)
                    == transform_bits
            );
        }

        // Compare the entire live session, including its VM, with the old
        // transaction at the actual boundary, then resume both ways of saving.
        let mut control = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
        )?;
        let assets = control.host.characters.get_mut(&id).unwrap();
        previous_service(
            &mut assets.geometry,
            &mut assets.model,
            control.simulation.characters.get_mut(&id).unwrap(),
            &mut control.host.mesh_pool,
        )?;
        let assets = session.host.characters.get_mut(&id).unwrap();
        ensure!(
            assets.geometry.service_resident(
                &mut assets.model,
                session.simulation.characters.get_mut(&id).unwrap(),
                &assets.avatar,
                &mut session.host.mesh_pool
            )? == Some(vec![])
        );
        ensure!(
            session.simulation == control.simulation
                && session.host.snapshot() == control.host.snapshot()
        );
        let mut resumed = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
        )?;
        ensure!(
            session.simulation == resumed.simulation
                && session.host.snapshot() == resumed.host.snapshot()
        );
        for _ in 0..3 {
            let progress = session.step()?;
            ensure!(progress == control.step()? && progress == resumed.step()?);
            ensure!(
                session.simulation == control.simulation
                    && session.host.snapshot() == control.host.snapshot()
            );
            ensure!(
                session.simulation == resumed.simulation
                    && session.host.snapshot() == resumed.host.snapshot()
            );
        }
        println!(
            "{source}: ModelBounds matches old transaction, rejects unpublished transforms atomically and resumes identically"
        );
    }
    Ok(())
}
