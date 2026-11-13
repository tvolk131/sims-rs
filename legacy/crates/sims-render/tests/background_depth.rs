//! The Iced room shader uses the GPU renderer, including its censor fallback.
#[path = "support/offscreen.rs"]
mod offscreen;

#[test]
#[ignore = "requires an offscreen GPU adapter"]
fn iced_gpu_background_has_no_clear_color_dashes_with_or_without_censor() -> anyhow::Result<()> {
    for (width, height) in [(1184, 803), (1320, 750)] {
        let color = [112, 112, 112, 255];
        let mut scene = sims_render::software::rgba_scene(
            width,
            height,
            color.repeat((width * height) as usize),
        )?;
        for vertex in &mut scene.vertices {
            vertex.position[2] = 1.;
        }
        for censored in [false, true] {
            if censored {
                // An empty region invokes exactly the software-composited
                // frame upload used by Iced, without altering its pixels.
                scene.censors.push(sims_render::censor::Region {
                    rect: [0; 4],
                    depth: 0.,
                    seed: 1,
                });
            }
            let frame = offscreen::render(&scene)?;
            let missing = frame.pixels().filter(|pixel| pixel.0 != color).count();
            assert_eq!(missing, 0, "{width}x{height}, censor={censored}");
        }
    }
    Ok(())
}
