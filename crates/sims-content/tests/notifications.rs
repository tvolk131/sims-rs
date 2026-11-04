use anyhow::{Result, ensure};
use sims_content::notifications::{Saved, Software};
use sims_sim::person_notification::{
    Style,
    currency::Locale,
    display::{Execution, Request},
};

fn draw(p: &mut Software, text: &[u8], style: Style, now: u32, restore: bool) -> Result<()> {
    let colors = Software::colors(style);
    let mut e =
        Execution::new(text.to_vec(), colors[0], colors[1], true).map_err(anyhow::Error::msg)?;
    p.saved.entries.entry(196).or_default();
    loop {
        let request = e
            .next(&mut p.saved.entries.get_mut(&196).unwrap().cache)
            .map_err(anyhow::Error::msg)?;
        let Some(request) = request else {
            break;
        };
        if restore {
            let bytes = serde_json::to_vec(&(&e, &p.saved))?;
            let (saved_e, saved_p): (Execution, Saved) = serde_json::from_slice(&bytes)?;
            ensure!(saved_e == e && saved_p == p.saved);
            e = saved_e;
            *p = Software::restore(saved_p)?;
        }
        let response = p.service(196, &request, now)?;
        e.acknowledge(&mut p.saved.entries.get_mut(&196).unwrap().cache, response)
            .map_err(anyhow::Error::msg)?;
        p.validate()?;
        if matches!(request, Request::Lock) {
            ensure!(p.saved.entries[&196].surface.as_ref().unwrap().locked);
        }
    }
    ensure!(e.is_complete());
    Ok(())
}

#[test]
fn notification_pixels_resume_and_expire() -> Result<()> {
    let mut direct = Software::system_font(Locale {
        language: 1,
        real_currency: false,
    })?;
    let mut resumed = Software::restore(direct.saved.clone())?;
    let mut pictures = Vec::new();
    for (text, style, tick) in [
        (b"$5".as_slice(), Style::Default, 180072),
        (b"100".as_slice(), Style::Yellow, 180173),
        (b"50".as_slice(), Style::Magenta, 180274),
    ] {
        draw(&mut direct, text, style, tick, false)?;
        draw(&mut resumed, text, style, tick, true)?;
        ensure!(
            direct.saved == resumed.saved,
            "restored notification pixels changed"
        );
        let entry = &direct.saved.entries[&196];
        ensure!(
            entry.cache.as_ref().unwrap().clock == tick && entry.cache.as_ref().unwrap().active
        );
        let surface = entry.surface.as_ref().unwrap();
        ensure!(
            surface.text == text
                && surface
                    .rgba
                    .chunks_exact(4)
                    .any(|px| px != Software::colors(style)[0].to_be_bytes()),
            "notification contains no drawn glyphs"
        );
        let image = direct.image(196)?;
        ensure!(
            image.width()
                == (entry.cache.as_ref().unwrap().rect[2] - entry.cache.as_ref().unwrap().rect[0])
                    as u32
        );
        ensure!(
            image == resumed.image(196)?,
            "restored visible rectangle changed"
        );
        pictures.push(image);
    }
    let unchanged = direct.saved.entries[&196].clone();
    draw(&mut direct, b"51", Style::Magenta, 180275, false)?;
    ensure!(
        direct.saved.entries[&196] == unchanged,
        "clock gate redrew notification"
    );
    ensure!(direct.present(196, 180373).is_some());
    ensure!(direct.present(196, 180374).is_none());
    ensure!(!direct.saved.entries.contains_key(&196));
    let mut broken = resumed.saved.clone();
    broken.font.bytes[0] ^= 1;
    ensure!(Software::restore(broken).is_err(), "changed font accepted");
    let mut broken = resumed.saved.clone();
    broken
        .entries
        .get_mut(&196)
        .unwrap()
        .surface
        .as_mut()
        .unwrap()
        .rgba
        .pop();
    ensure!(
        Software::restore(broken).is_err(),
        "missing pixel storage accepted"
    );
    let mut gallery = image::RgbaImage::from_pixel(600, 160, image::Rgba([28, 35, 40, 255]));
    for (i, picture) in pictures.iter().enumerate() {
        let scaled = image::imageops::resize(
            picture,
            picture.width() * 4,
            picture.height() * 4,
            image::imageops::FilterType::Nearest,
        );
        image::imageops::overlay(&mut gallery, &scaled, 25 + i as i64 * 195, 40);
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::create_dir_all(root.join("cache"))?;
    gallery.save(root.join("cache/notification-software-gallery.png"))?;
    Ok(())
}
