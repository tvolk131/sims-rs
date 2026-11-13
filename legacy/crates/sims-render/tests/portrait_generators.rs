use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::json;
use sims_content::{
    Index,
    dialog::portrait::{Layout, Rect, generators},
};
use sims_render::portrait::{
    blit, generate,
    images::{self, Raster, Software},
    resize::Format,
};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::pose::portrait::view::Input;
use std::collections::BTreeMap;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_rgb_blit_clipping_matches_original_dispatch() {
    #[derive(Deserialize)]
    struct Input {
        size: [i32; 2],
        from: Rect,
        to: Rect,
        clip: Option<Rect>,
        success: bool,
    }
    #[derive(Deserialize)]
    struct Case {
        input: Input,
        original: serde_json::Value,
    }
    #[derive(Deserialize)]
    struct Report {
        cases: Vec<Case>,
    }
    let report: Report = serde_json::from_str(include_str!("fixtures/portrait-blit.json")).unwrap();
    assert_eq!(report.cases.len(), 224);
    for (n, c) in report.cases.into_iter().enumerate() {
        let planned = blit::rectangles(c.input.size, c.input.from, c.input.to, c.input.clip);
        assert_eq!(
            json!({"success":planned.is_none()||c.input.success,"calls":planned.into_iter().map(|(a,b)|[a,b]).collect::<Vec<_>>()}),
            c.original,
            "case {n}"
        );
    }
}

#[test]
fn composed_rows_use_first_face_and_preserve_packed_transparency() -> Result<()> {
    let format = Format::Rgb565;
    let key = format.encode([255, 0, 255]);
    let red = format.encode([255, 0, 0]);
    let green = format.encode([0, 255, 0]);
    let blue = format.encode([0, 0, 255]);
    let raster = |pixels: Vec<u16>, key| Raster {
        format,
        image: sims_render::portrait::resize::Image {
            width: pixels.len(),
            height: 1,
            pixels,
            alpha: None,
        },
        key,
    };
    let background = raster(vec![blue; 4], None);
    for (first, expected) in [(red, red), (key, blue)] {
        let faces = raster(vec![first, green, blue], Some(key));
        let output = images::compose(
            Layout([1, 1, 0, 0, 1, 1]),
            &faces,
            &background,
            u32::from(key),
        )?;
        ensure!(output.success);
        let output = output.image.unwrap();
        assert_eq!((output.image.width, output.image.height), (4, 3));
        assert_eq!(output.image.pixels, vec![expected; 12]);
        assert_eq!(output.key, Some(key));
        assert_eq!(faces.image.pixels, vec![first, green, blue]);
        assert_eq!(background.image.pixels, vec![blue; 4]);
    }
    Ok(())
}

#[test]
#[ignore = "requires private original household and UI templates; CPU rendering"]
fn original_household_generates_owned_portraits_and_resumes_identically() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-portrait-prepared-session.json"),
    )?)?;
    let mut session = Session::restore(&index, saved)?;
    session
        .host
        .prepare_portrait(&mut session.simulation, &session.program, 196)?;
    let before = session.save();
    let mut resumed = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&before)?)?,
    )?;
    let mut templates = BTreeMap::new();
    for id in [108, 4600, 4602, 9000, 9001] {
        templates.insert(id, images::template(&index, id, Format::Rgb565)?.unwrap());
    }
    let original_templates = templates.clone();
    // 4ff31a's palette initializer: packed display-format magenta, not RGB888.
    let key = u32::from(Format::Rgb565.encode([255, 0, 255]));
    let mut gallery = image::RgbaImage::from_pixel(440, 7 * 80, image::Rgba([24, 32, 37, 255]));
    let mut evidence = vec![];
    for (row, slot) in [0, 2, 3, 4, 5, 6].into_iter().enumerate() {
        let view = Input {
            viewport: [0, 0, 800, 600],
            center: [344, 232],
            zoom: 1,
            rotation: 0,
        };
        let initial = (slot == 3).then(|| templates[&108].clone());
        let mut calls = vec![];
        let mut host = Software::new(
            Format::Rgb565,
            templates.clone(),
            initial.clone(),
            |request| {
                calls.push(request);
                generate::head(&mut session, 196, view, Format::Rgb565, None, request)
            },
        )?;
        let returned =
            generators::generate(slot, Layout([45, 45, 5, 2, 35, 41]), 2, key, 196, &mut host)
                .map_err(anyhow::Error::msg)?;
        let output = host.finish(returned)?;
        ensure!(output.success, "portrait generator {slot} failed");
        let pixels = output.image.unwrap();
        if slot == 0 {
            let atlas = images::compose(
                Layout([45, 45, 5, 2, 35, 41]),
                &pixels,
                &templates[&4600],
                key,
            )?;
            let repeated = images::compose(
                Layout([45, 45, 5, 2, 35, 41]),
                &serde_json::from_slice(&serde_json::to_vec(&pixels)?)?,
                &templates[&4600],
                key,
            )?;
            ensure!(atlas.success && repeated.success && atlas.image == repeated.image);
            let atlas = atlas.image.unwrap();
            ensure!((atlas.image.width, atlas.image.height) == (180, 135));
            image::imageops::overlay(&mut gallery, &atlas.rgba()?, 240, 8);
            evidence.push(json!({"slot":1,"dimensions":[180,135],"restored_identically":true}));
        }
        let saved: Raster = serde_json::from_slice(&serde_json::to_vec(&pixels)?)?;
        ensure!(saved == pixels);
        let mut second = Software::new(Format::Rgb565, templates.clone(), initial, |request| {
            generate::head(&mut resumed, 196, view, Format::Rgb565, None, request)
        })?;
        let returned = generators::generate(
            slot,
            Layout([45, 45, 5, 2, 35, 41]),
            2,
            key,
            196,
            &mut second,
        )
        .map_err(anyhow::Error::msg)?;
        let restored = second.finish(returned)?;
        ensure!(restored.success && restored.image.as_ref() == Some(&pixels));
        ensure!(
            session.simulation == before.simulation.state
                && resumed.simulation == session.simulation
        );
        ensure!(session.host.snapshot() == resumed.host.snapshot());
        ensure!(templates == original_templates);
        let rgba = pixels.rgba()?;
        ensure!(rgba.width() <= 220 && rgba.height() <= 80);
        image::imageops::overlay(&mut gallery, &rgba, 8, row as i64 * 80 + 8);
        evidence.push(json!({"slot":slot,"dimensions":[pixels.image.width,pixels.image.height],"key":pixels.key,"render_requests":calls,"restored_identically":true}));
        println!(
            "slot {slot}: original generator produced {}x{} image; owned pixels and source state survive restore",
            pixels.image.width, pixels.image.height
        );
    }
    gallery.save(root.join("cache/original-generated-portraits.png"))?;
    std::fs::write(
        root.join("cache/original-generated-portraits.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    Ok(())
}
