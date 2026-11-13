use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::portrait::{cache::Images, resize::Format};
use sims_vm::image_cache::{
    State,
    object_speech::{Graphic, Request, Retrieval},
};

#[path = "../../sims-content/tests/support/portrait.rs"]
#[allow(dead_code)]
mod support;

#[test]
#[ignore = "requires original Complete Collection assets; headless object images"]
fn original_tulips_speech_bitmaps_load_and_restore_without_drawing_or_writes() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(std::env::var("SIMS_OBJECT_SPEECH_OUTPUT")?);
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source = "GameData_ALL_Recursive/Objects/Objects.far::FlowersOutdoor.iff";
    let original = index.resources[index.find(source)?].read()?;
    let mut report = vec![];
    for zoom in [2, 3] {
        let mut pair = vec![];
        for restoring in [false, true] {
            let mut catalog = support::catalog(&index)?;
            let resource = catalog
                .portrait_resource(0x65274a4f)
                .map_err(anyhow::Error::msg)?
                .context("Tulips absent")?;
            let before_overlay = catalog.resources.overlay().map_err(anyhow::Error::msg)?;
            let mut graphic = Graphic {
                kind: 1,
                resource: Some(resource),
                images: [None; 2],
            };
            let mut state = State::default();
            let mut images = Images::default();
            let mut operation = Retrieval::new(zoom, None);
            let mut requests = vec![];
            while let Some(request) = operation
                .advance(&mut state, &mut graphic, false, &mut |e, s| {
                    images.event(e, s)
                })
                .map_err(anyhow::Error::msg)?
            {
                ensure!(
                    matches!(request, Request::Definition { .. } | Request::Load { .. }),
                    "saved Tulips unexpectedly needs {request:?}"
                );
                requests.push(request);
                if restoring {
                    (operation, state, graphic, images) = serde_json::from_slice(
                        &serde_json::to_vec(&(&operation, &state, &graphic, &images))?,
                    )?;
                }
                ensure!(sims_render::object_speech::service(
                    &index,
                    &mut catalog,
                    &mut images,
                    &mut state,
                    &mut graphic,
                    &mut operation,
                    Format::Rgb565
                )?);
            }
            ensure!(
                requests
                    == vec![
                        Request::Definition { resource },
                        Request::Load {
                            resource,
                            chunk: 4002
                        },
                        Request::Load {
                            resource,
                            chunk: 6002
                        }
                    ]
            );
            let (success, mut owner) = operation.finish().map_err(anyhow::Error::msg)?;
            ensure!(success);
            let image = owner.context("Tulips speech image absent")?;
            let frame = images.get(image)?.rgba()?;
            ensure!(frame.dimensions() == if zoom == 2 { (16, 16) } else { (32, 32) });
            if restoring {
                frame.save(output.join(format!("tulips-zoom{zoom}.png")))?;
            }
            images.release(&mut state, &mut owner)?;
            let before = (state.clone(), graphic.clone(), images.clone());
            // Even force=true cannot trigger regeneration through this getter
            // when the small cached wrapper exists.
            let mut cached = Retrieval::new(zoom, None);
            ensure!(
                cached
                    .advance(&mut state, &mut graphic, true, &mut |e, s| images
                        .event(e, s))
                    .map_err(anyhow::Error::msg)?
                    .is_none()
            );
            let (success, mut cached_owner) = cached.finish().map_err(anyhow::Error::msg)?;
            ensure!(success && cached_owner == Some(image));
            images.release(&mut state, &mut cached_owner)?;
            ensure!(before == (state.clone(), graphic.clone(), images.clone()));
            ensure!(catalog.resources.overlay().map_err(anyhow::Error::msg)? == before_overlay);
            pair.push((state, graphic, images, frame));
        }
        ensure!(pair[0] == pair[1]);
        report.push(serde_json::json!({"zoom":zoom,"state_and_pixels_restore":true,"resource_writes":false,"generated_images":false,"dimensions":pair[0].3.dimensions()}));
    }
    ensure!(
        index.resources[index.find(source)?].read()? == original,
        "original FlowersOutdoor bytes changed"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires original assets; explicitly withheld object-render service"]
fn missing_object_image_stays_pending_at_actual_generation() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut catalog = support::catalog(&index)?;
    let resource = catalog
        .portrait_resource(0x65274a4f)
        .map_err(anyhow::Error::msg)?
        .unwrap();
    let mut graphic = Graphic {
        kind: 1,
        resource: Some(resource),
        images: [None; 2],
    };
    let mut state = State::default();
    let mut images = Images::default();
    let mut op = Retrieval::new(2, None);
    let mut observed = false;
    // Explicit force flag exercises the original regeneration branch without
    // deleting/rewriting any installed bitmap or completing its draw request.
    while let Some(request) = op
        .advance(&mut state, &mut graphic, true, &mut |e, s| {
            images.event(e, s)
        })
        .map_err(anyhow::Error::msg)?
    {
        let before = (op.clone(), state.clone(), graphic.clone(), images.clone());
        let handled = sims_render::object_speech::service(
            &index,
            &mut catalog,
            &mut images,
            &mut state,
            &mut graphic,
            &mut op,
            Format::Rgb565,
        )?;
        if !handled {
            ensure!(matches!(request, Request::Generate { .. }));
            ensure!(before == (op.clone(), state.clone(), graphic.clone(), images.clone()));
            ensure!(op.clone().finish().is_err());
            (op, state, graphic, images) = serde_json::from_slice(&serde_json::to_vec(&before)?)?;
            op.validate(&state, &graphic).map_err(anyhow::Error::msg)?;
            ensure!(!sims_render::object_speech::service(
                &index,
                &mut catalog,
                &mut images,
                &mut state,
                &mut graphic,
                &mut op,
                Format::Rgb565
            )?);
            observed = true;
            break;
        }
    }
    ensure!(observed);
    Ok(())
}
