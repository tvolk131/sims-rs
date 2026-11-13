//! Software frontend for a single-part catalog object. Selection, sprite
//! placement and fit follow 43878f/4384c8. Pixel decoding/composition is the Rust
//! presentation backend: native SPR2 blitter/lighting/pixel parity is UNVERIFIED.
//! Multipart view/depth projection and legacy pixel codecs remain explicit
//! errors. This endpoint has no simulation access and never creates an object.
use super::fit::{Definition, Fit, Output, Rect, Request};
use crate::portrait::{
    images::Raster,
    resize::{Format, Image},
};
use anyhow::{Context, Result, ensure};
use sims_content::{object_catalog::Catalog, object_graphics::Sources};
use sims_formats::sprite::{DrawSprite, Frame};

struct Layer {
    sprite: DrawSprite,
    frame: Frame,
    origin: [i32; 2],
}
struct Plan {
    bounds: Rect,
    layers: Vec<Layer>,
}

/// 438ae4..438b58: DGRP offsets are anchored at the native frame's bottom,
/// not its SPR2 pixel-origin fields. __ftol truncates after adding 0.5; in
/// particular negative heights do not use Rust's round-to-nearest rule.
pub fn origin(sprite: &DrawSprite, rect: [i16; 4], request: Request) -> Result<[i32; 2]> {
    let mut y = sprite
        .y
        .wrapping_sub(i32::from(rect[2]))
        .wrapping_add(request.offset[1]);
    if sprite.object[2] != 0.0 {
        let scale = 1i32.wrapping_shl(request.zoom.wrapping_add(1) as u32);
        let z = f64::from(sprite.object[2]) * f64::from(scale) + 0.5;
        ensure!(
            z.is_finite() && z >= f64::from(i32::MIN) && z <= f64::from(i32::MAX),
            "software preview height conversion out of range"
        );
        y = y.wrapping_sub(z.trunc() as i32);
    }
    Ok([sprite.x.wrapping_add(request.offset[0]), y])
}

fn plan(
    sources: &mut Sources<'_, '_>,
    handle: u32,
    selector: i32,
    request: Request,
) -> Result<Plan> {
    let mut result = Plan {
        bounds: [i32::MAX, i32::MAX, i32::MIN, i32::MIN],
        layers: vec![],
    };
    let Some(view) = sources.selected_view(handle, selector, request.facing, request.zoom)? else {
        return Ok(result);
    };
    for sprite in view.sprites {
        let frame = sprite.frame as i32;
        if frame >= sources.frame_count(handle, sprite.id)? {
            continue;
        }
        let metadata = sources
            .frame(handle, sprite.id, frame)?
            .context("sprite metadata absent")?;
        let [top, left, bottom, right] = metadata.rect;
        if top >= bottom || left >= right {
            continue;
        }
        let [x, y] = origin(&sprite, metadata.rect, request)?;
        // Captured import metadata for 438b4c identifies OffsetRect. Min/max
        // union at 422665 includes each translated rectangle in sprite order.
        let rect = [
            i32::from(left).wrapping_add(x),
            i32::from(top).wrapping_add(y),
            i32::from(right).wrapping_add(x),
            i32::from(bottom).wrapping_add(y),
        ];
        for n in 0..2 {
            result.bounds[n] = result.bounds[n].min(rect[n]);
            result.bounds[n + 2] = result.bounds[n + 2].max(rect[n + 2]);
        }
        if !request.bounds_only {
            ensure!(
                sprite.flags & !5 == 0,
                "software preview sprite flags unsupported"
            );
            let frame = sources.decode_spr2(handle, sprite.id, frame)?;
            ensure!(
                frame.flags & 2 != 0,
                "software preview sprite has no depth channel"
            );
            result.layers.push(Layer {
                sprite,
                frame,
                origin: [x, y],
            });
        }
    }
    Ok(result)
}

/// Returns a 125x127 owned surface with the original 120x120 published cell.
/// Color packing/key and alpha composition are frontend policy, as in the
/// existing software portrait endpoint; no Windows pixel equivalence is implied.
pub fn dialog(
    catalog: &mut Catalog<'_>,
    handle: u32,
    format: Format,
    key: u16,
) -> Result<(Raster, Output)> {
    let mut sources = Sources::new(catalog);
    let definition = sources.definition(handle)?;
    ensure!(
        definition.group == 0,
        "multipart object preview projection unsupported"
    );
    let mut fit = Fit::new(
        Definition {
            group: definition.group,
            subindex: definition.subindex,
            direction: definition.direction,
        },
        [120, 120],
        None,
    );
    let mut raster = Raster {
        format,
        key: Some(key),
        image: Image {
            width: 125,
            height: 127,
            pixels: vec![key; 125 * 127],
            alpha: Some(vec![0; 125 * 127]),
        },
    };
    while let Some(request) = fit.pending() {
        let plan = plan(
            &mut sources,
            handle,
            i32::from(definition.preview_index),
            request,
        )?;
        if !request.bounds_only {
            paint(&plan.layers, &mut raster)?;
        }
        fit.respond(request, plan.bounds)
            .map_err(anyhow::Error::msg)?;
    }
    raster.validate()?;
    Ok((raster, fit.finish().map_err(anyhow::Error::msg)?))
}

fn paint(layers: &[Layer], raster: &mut Raster) -> Result<()> {
    let mut depth = vec![255u8; raster.image.pixels.len()];
    for Layer {
        sprite,
        frame,
        origin: [x, y],
    } in layers
    {
        for fy in 0..frame.height as usize {
            for fx in 0..frame.width as usize {
                let dx = i64::from(*x) + fx as i64;
                let dy = i64::from(*y) + fy as i64;
                if !(0..120).contains(&dx) || !(0..120).contains(&dy) {
                    continue;
                }
                let sx = if sprite.flags & 1 != 0 {
                    frame.width as usize - 1 - fx
                } else {
                    fx
                };
                let source = fy * frame.width as usize + sx;
                let target = dy as usize * raster.image.width + dx as usize;
                let rgba = &frame.rgba[source * 4..source * 4 + 4];
                let a = u32::from(rgba[3]);
                let z = frame.depth[source];
                if a == 0 || z == 255 || z > depth[target] {
                    continue;
                }
                let old = raster.format.decode(raster.image.pixels[target]);
                let alpha = raster
                    .image
                    .alpha
                    .as_mut()
                    .context("preview alpha plane absent")?;
                let previous = u32::from(alpha[target]);
                let total = a * 255 + previous * (255 - a);
                let rgb = std::array::from_fn(|n| {
                    ((u32::from(rgba[n]) * a * 255 + u32::from(old[n]) * previous * (255 - a))
                        / total) as u8
                });
                raster.image.pixels[target] = raster.format.encode(rgb);
                alpha[target] = (total / 255) as u8;
                depth[target] = z;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sprite_origin_uses_bottom_and_original_negative_height_rounding() -> Result<()> {
        let mut s = DrawSprite {
            id: 1,
            frame: 0,
            x: -27,
            y: 13,
            flags: 0,
            object: [0., 0., 0.],
        };
        let request = Request {
            zoom: 2,
            facing: 4,
            offset: [59, 102],
            bounds_only: true,
        };
        assert_eq!(origin(&s, [0, 0, 110, 56], request)?, [32, 5]);
        s.object[2] = -0.25;
        assert_eq!(origin(&s, [0, 0, 110, 56], request)?, [32, 6]);
        s.object[2] = 0.25;
        assert_eq!(origin(&s, [0, 0, 110, 56], request)?, [32, 3]);
        s.object[2] = f32::NAN;
        assert!(origin(&s, [0, 0, 110, 56], request).is_err());
        Ok(())
    }
    #[test]
    fn software_pixels_clip_flip_and_later_equal_depth_wins() -> Result<()> {
        let layer = |colors: Vec<u8>, flags, origin| Layer {
            sprite: DrawSprite {
                id: 1,
                frame: 0,
                x: 0,
                y: 0,
                flags,
                object: [0.; 3],
            },
            frame: Frame {
                width: 2,
                height: 1,
                x: 500,
                y: -500,
                flags: 2,
                rgba: colors,
                depth: vec![10, 20],
            },
            origin,
        };
        let red = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let blue = vec![0, 0, 255, 255, 255, 255, 255, 255];
        let mut raster = Raster {
            format: Format::Rgb565,
            key: Some(0xf81f),
            image: Image {
                width: 125,
                height: 127,
                pixels: vec![0xf81f; 125 * 127],
                alpha: Some(vec![0; 125 * 127]),
            },
        };
        paint(
            &[layer(red, 1, [-1, 119]), layer(blue, 1, [-1, 119])],
            &mut raster,
        )?;
        assert_eq!(raster.image.pixels[119 * 125], 0x001f);
        assert_eq!(
            raster.image.pixels.iter().filter(|&&p| p != 0xf81f).count(),
            1
        );
        Ok(())
    }
}
