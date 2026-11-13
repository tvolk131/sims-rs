use anyhow::{Context, Result, ensure};
use sims_content::{
    Index,
    avatar_installation::{Binding, Installation, complete_collection_asset_bindings},
    object_catalog::{Catalog, Settings},
    object_resources::{Overlay, Resources},
};
use sims_runtime::original_session::{Options, Saved, Session, View};
use sims_sim::{
    lot_route::path::Policy, object_scan::Paths, placement::SearchView, portrait_person::Service,
};

#[allow(dead_code)] // Shared by tests that choose different resource-access policies.
pub fn catalog(index: &Index) -> Result<Catalog<'_>> {
    catalog_with_overlay(index, Overlay::default())
}

pub fn catalog_with_overlay(index: &Index, overlay: Overlay) -> Result<Catalog<'_>> {
    let people = b"./UserData2/Characters/".to_vec();
    let mut bindings = complete_collection_asset_bindings(&index.root);
    bindings.push(Binding {
        directory: people.clone(),
        physical: index.root.join("UserData2/Characters"),
    });
    let mut installation = Installation::new(index, bindings)?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    installation.register(&people, false, true)?;
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        people,
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|n| initial[3 + n].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
    };
    let mut resources = Resources::with_overlay(
        index,
        installation.directories()?,
        installation.directory_search(),
        overlay,
    )
    .map_err(anyhow::Error::msg)?;
    let global = resources
        .acquire(&[initial[0].as_slice(), b"Global"].concat(), 0)
        .map_err(anyhow::Error::msg)?
        .context("global resource absent")?;
    let mut catalog = Catalog::new(
        resources,
        Settings {
            filter_flagged: true,
            global_directory: initial[0].clone(),
            global_people: 0,
            global: Some(global),
        },
    );
    installation.scan_objects(&paths, 255, 0, false, &mut catalog)?;
    Ok(catalog)
}

pub fn capture(index: &Index, root: &std::path::Path) -> Result<Session> {
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
        "cache/neighborhood-input-experiments/mashuga-live-reset-removal-v81/original-live-character-final.json"))?)?;
    let source = data["source"].as_str().context("capture source absent")?;
    let house = sims_content::saves::House::load(index, source)?;
    Session::restore(
        index,
        Saved {
            version: 1,
            managed: None,
            source: source.into(),
            source_hashes: house.sources,
            options: Options {
                routing: Policy::InferredRectangles,
                ..Default::default()
            },
            modules: serde_json::from_value(data["program_modules"].clone())?,
            animations: serde_json::from_value(data["program_animations"].clone())?,
            native_animation_absences: Default::default(),
            // This historical capture predates the optional Sprites.iff
            // dependency; retain its recorded program identity on restore.
            headline_catalog: false,
            normalized_object_definitions: Default::default(),
            simulation: serde_json::from_value(data["simulation"].clone())?,
            assets: serde_json::from_value(data["assets"].clone())?,
            view: View::default(),
        },
    )
}

pub fn supply(
    session: &mut Session,
    catalog: &mut Catalog<'_>,
    service: Service,
    projected: [i32; 2],
) -> Result<()> {
    match service {
        Service::Resource { guid } => {
            let handle = catalog
                .portrait_resource(guid)
                .map_err(anyhow::Error::msg)?
                .context("resource absent")?;
            let definition = catalog
                .portrait_definition(handle)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                definition.source == "UserData2/Characters/User00000.iff"
                    && definition.object == 128
                    && definition.guid == guid
            );
            let owner = session.simulation.portrait_person.clone();
            let mut wrong = definition.clone();
            wrong.slots += 1;
            ensure!(
                session
                    .simulation
                    .supply_portrait_person_resource(&session.program, Some((handle, wrong)))
                    .is_err()
            );
            ensure!(session.simulation.portrait_person == owner);
            session
                .simulation
                .supply_portrait_person_resource(&session.program, Some((handle, definition)))
                .map_err(anyhow::Error::msg)?;
        }
        Service::CameraOrigin => session
            .simulation
            .supply_portrait_person_camera(
                SearchView {
                    viewport: [0, 0, 800, 600],
                    bounds: [0, 64, 0, 64],
                    rotation: 0,
                },
                projected,
            )
            .map_err(anyhow::Error::msg)?,
    }
    Ok(())
}
