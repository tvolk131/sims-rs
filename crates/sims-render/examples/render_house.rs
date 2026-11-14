//! Renders a house to PNG files: `render_house <install> <house> <out dir>
//! [zoom rotation walls level width height]...`. A development tool; the
//! images hold game art, so keep them out of the repository.
use sims_render::{Camera, Canvas, Lot, Walls, Zoom, draw, open_house, png};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, install, house, out, ..] = args.as_slice() else {
        return Err("usage: render_house <install> <house> <out dir>".into());
    };
    let mut opened = open_house(Path::new(install), Path::new(house))?;
    let lot = Lot::from_world(&opened.world)?;
    let (width, height) = (1024, 768);
    for zoom in Zoom::ALL {
        for rotation in 0..4u8 {
            for (walls, name) in [
                (Walls::Up, "up"),
                (Walls::Cutaway, "cut"),
                (Walls::Down, "down"),
            ] {
                let mut camera = Camera {
                    zoom,
                    rotation,
                    level: 2,
                    origin: [0, 0],
                    walls,
                };
                camera.center_on(lot.built_center(), [width / 2, height / 2]);
                let mut canvas = Canvas::new(width as usize, height as usize, [0, 0, 0]);
                let stats = draw(
                    &opened.world,
                    &lot,
                    &mut opened.assets,
                    &camera,
                    &mut canvas,
                );
                let path =
                    Path::new(out).join(format!("z{}r{}{}.png", zoom.level(), rotation, name));
                std::fs::write(&path, png::encode(&canvas))?;
                println!("{} {:?}", path.display(), stats);
            }
        }
    }
    Ok(())
}
