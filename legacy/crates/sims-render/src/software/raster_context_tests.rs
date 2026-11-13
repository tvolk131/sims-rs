//! Actual raster/context pixels through the existing software scene composer.
//! No screen acquisition, native HDC/API success or household/window claim.

use sims_content::native_bootstrap::{
    Memory,
    services::{program_raster as raster, program_raster_context as contexts},
};

#[test]
fn raster_context_pixels_survive_source_close_and_render_in_existing_software_scene() {
    let memory = Memory::new(8192).unwrap();
    let mut device = raster::Device::new(raster::Limits {
        max_width: 64, max_height: 64, max_bitmap_bytes: 4096,
        max_live_bytes: 16384, max_bitmaps: 16, max_selections: 16,
    }).unwrap();
    let source_bitmap = device.create_bitmap(raster::PixelFormat::Rgba8Straight, 5, 4).unwrap();
    let target_bitmap = device.create_bitmap(raster::PixelFormat::Rgba8Straight, 4, 3).unwrap();
    let target_observation = device.observe(&target_bitmap).unwrap();
    let mut registry = contexts::Registry::new(&memory, device, contexts::Policy::PrivateRasterV1, 8).unwrap();
    let source = registry.attach_source(&memory, &source_bitmap).unwrap();
    registry.set_brush(&memory, &source, [10, 20, 30, 255]).unwrap();
    registry.fill_rect(&memory, &source, raster::Rect { x: 0, y: 0, width: 5, height: 4 }).unwrap();
    let context = registry.create_compatible(&memory, Some(&source)).unwrap();
    assert_eq!(registry.state(&memory, &context).unwrap().brush, [255, 255, 255, 255]);
    let default = registry.selected_bitmap(&memory, &context).unwrap();
    let initial = registry.readback_rgba(&memory, &context).unwrap();
    assert_eq!((initial.width, initial.height), (1, 1));
    assert_eq!(initial.pixels, [0, 0, 0, 255]);
    let previous = registry.select_bitmap(&memory, &context, &target_bitmap).unwrap();
    assert!(previous.same_bitmap(&default));
    registry.set_brush(&memory, &context, [40, 80, 120, 255]).unwrap();
    registry.fill_rect(&memory, &context, raster::Rect { x: 0, y: 0, width: 4, height: 3 }).unwrap();
    registry.set_brush(&memory, &context, [200, 100, 10, 255]).unwrap();
    registry.fill_rect(&memory, &context, raster::Rect { x: 1, y: 1, width: 2, height: 1 }).unwrap();
    registry.close(&memory, &source).unwrap();
    assert_eq!(registry.bitmap_info(&memory, &source_bitmap).unwrap().width, 5);
    let retained = registry.snapshot(&memory).unwrap().device.live_bitmap_bytes;
    drop(source_bitmap);
    let released = registry.snapshot(&memory).unwrap().device.live_bitmap_bytes;
    assert!(retained >= released + 5 * 4 * 4, "actual source pixel backing was not released");
    let rgba = registry.readback_rgba(&memory, &context).unwrap();
    let expected_bytes = rgba.pixels.clone();
    // Output pixels are presentation data; no image/snapshot is adopted as an
    // owning context/device or evidence of the application's current screen.
    let scene = super::rgba_scene(rgba.width, rgba.height, rgba.pixels).unwrap();
    let rendered = super::render(&scene);
    assert_eq!(rendered.dimensions(), (4, 3));
    for y in 0..3 {
        for x in 0..4 {
            let expected = if y == 1 && (1..3).contains(&x) {
                [200, 100, 10, 255]
            } else {
                [40, 80, 120, 255]
            };
            assert_eq!(rendered.get_pixel(x, y).0, expected, "pixel ({x},{y})");
        }
    }
    let selected = registry.restore_default(&memory, &context).unwrap();
    assert!(selected.same_bitmap(&target_observation));
    assert!(registry.selected_bitmap(&memory, &context).unwrap().same_bitmap(&default));
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, [0, 0, 0, 255]);
    registry.close(&memory, &context).unwrap();
    // Context cleanup leaves the caller-owned target intact and selectable.
    let reused = registry.attach_source(&memory, &target_bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &reused).unwrap().pixels, expected_bytes);
    registry.close(&memory, &reused).unwrap();
}

#[test]
fn raster_context_transparent_readback_blends_each_shared_edge_pixel_once() {
    for (width, height) in [(2, 2), (4, 4), (5, 3)] {
        let memory = Memory::new(8192).unwrap();
        let mut device = raster::Device::new(raster::Limits {
            max_width: 64, max_height: 64, max_bitmap_bytes: 4096,
            max_live_bytes: 16384, max_bitmaps: 16, max_selections: 16,
        }).unwrap();
        let bitmap = device.create_bitmap(raster::PixelFormat::Rgba8Straight, width, height).unwrap();
        let mut registry = contexts::Registry::new(&memory, device, contexts::Policy::PrivateRasterV1, 8).unwrap();
        let context = registry.attach_source(&memory, &bitmap).unwrap();
        registry.set_brush(&memory, &context, [255, 0, 0, 128]).unwrap();
        registry.fill_rect(&memory, &context, raster::Rect { x: 0, y: 0, width, height }).unwrap();
        let rgba = registry.readback_rgba(&memory, &context).unwrap();
        assert_eq!(rgba.pixels, [255, 0, 0, 128].repeat((width * height) as usize));
        let base = super::rgba_scene(width, height, rgba.pixels).unwrap();
        // Both diagonals, both windings, and either triangle submission order
        // must cover each sample once, using actual semitransparent readback.
        for variant in 0..8 {
            let mut scene = base.clone();
            if variant & 4 != 0 {
                let v = &base.vertices;
                scene.vertices = vec![v[0], v[1], v[5], v[1], v[2], v[5]];
            }
            if variant & 1 != 0 {
                for triangle in scene.vertices.chunks_exact_mut(3) { triangle.swap(1, 2); }
            }
            if variant & 2 != 0 { scene.vertices.rotate_left(3); }
            let frame = super::render_with_background(&scene, [0, 0, 0, 255]);
            for (x, y, pixel) in frame.enumerate_pixels() {
                assert_eq!(pixel.0, [128, 0, 0, 255], "{width}x{height}, variant {variant}, pixel ({x},{y})");
            }
        }
        registry.close(&memory, &context).unwrap();
    }
}

#[test]
fn raster_context_distinct_overlapping_quads_still_blend_at_equal_depth() {
    let mut scene = super::rgba_scene(4, 4, [255, 0, 0, 128].repeat(16)).unwrap();
    scene.draws.push(scene.draws[0].clone());
    let frame = super::render_with_background(&scene, [0, 0, 0, 255]);
    for (x, y, pixel) in frame.enumerate_pixels() {
        assert_eq!(pixel.0, [192, 0, 0, 255], "overlap pixel ({x},{y})");
    }
}
