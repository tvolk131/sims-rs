//! Authored source controls only. UNRUN by the bounded implementation worker.
//! Parent must declare program_raster before running these exact unit filters.

use super::*;

fn limits() -> Limits {
    Limits {
        max_width: 64, max_height: 64, max_bitmap_bytes: 4096,
        max_live_bytes: 8192, max_bitmaps: 64, max_selections: 64,
    }
}

fn device() -> Device { Device::new(limits()).unwrap() }

fn selected(device: &mut Device, format: PixelFormat, width: u32, height: u32)
    -> (Bitmap, Selection)
{
    let bitmap = device.create_bitmap(format, width, height).unwrap();
    let mut selection = device.new_selection().unwrap();
    assert!(device.select(&mut selection, &bitmap).unwrap().is_none());
    (bitmap, selection)
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect { x, y, width, height }
}

fn pixel(image: &RgbaImage, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * image.width as usize + x) * 4;
    image.pixels[offset..offset + 4].try_into().unwrap()
}

fn raw(device: &Device, bitmap: &Bitmap) -> Vec<u8> {
    let state = device.lock().unwrap();
    let index = state.bitmap(&device.shared, bitmap).unwrap();
    state.bitmaps[index].pixels.as_ref().unwrap().clone()
}

fn assert_unsupported<T: fmt::Debug>(result: Result<T, Stop>) {
    assert!(matches!(result, Err(Stop::Unsupported(_))), "{result:?}");
}

fn assert_unavailable<T: fmt::Debug>(result: Result<T, Stop>) {
    assert!(matches!(result, Err(Stop::Unavailable(_))), "{result:?}");
}

// Ambiguous inference would reject this source if an owner acquired Clone.
trait AmbiguousIfClone<A> { fn check() {} }
impl<T: ?Sized> AmbiguousIfClone<()> for T {}
struct IsClone;
impl<T: ?Sized + Clone> AmbiguousIfClone<IsClone> for T {}

#[test]
fn owners_and_weak_observations_are_send_sync_without_owner_clone() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Device>();
    send_sync::<Bitmap>();
    send_sync::<Selection>();
    send_sync::<BitmapObservation>();
    let _ = <Device as AmbiguousIfClone<_>>::check;
    let _ = <Bitmap as AmbiguousIfClone<_>>::check;
    let _ = <Selection as AmbiguousIfClone<_>>::check;
}

#[test]
fn every_invalid_limit_is_refused_before_a_device_is_published() {
    let good = limits();
    for bad in [
        Limits { max_width: 0, ..good },
        Limits { max_width: 8193, ..good },
        Limits { max_height: 0, ..good },
        Limits { max_height: 8193, ..good },
        Limits { max_bitmap_bytes: 0, ..good },
        Limits { max_bitmap_bytes: MAX_BITMAP_BYTES + 1, ..good },
        Limits { max_live_bytes: 0, ..good },
        Limits { max_live_bytes: MAX_LIVE_BYTES + 1, ..good },
        Limits { max_live_bytes: good.max_bitmap_bytes - 1, ..good },
        Limits { max_bitmaps: 0, ..good },
        Limits { max_bitmaps: 4097, ..good },
        Limits { max_selections: 0, ..good },
        Limits { max_selections: 4097, ..good },
    ] {
        assert_unsupported(Device::new(bad));
    }
}

#[test]
fn configuration_boundaries_allocate_no_pixels_or_issuances() {
    for bound in [
        Limits { max_width: 1, max_height: 1, max_bitmap_bytes: 1,
            max_live_bytes: 1, max_bitmaps: 1, max_selections: 1 },
        Limits { max_width: 8192, max_height: 8192, max_bitmap_bytes: MAX_BITMAP_BYTES,
            max_live_bytes: MAX_LIVE_BYTES, max_bitmaps: 4096, max_selections: 4096 },
    ] {
        assert_eq!(Device::new(bound).unwrap().snapshot().unwrap(), Observation {
            bitmap_issuances: 0, selection_issuances: 0, live_bitmap_bytes: 0,
        });
    }
}

#[test]
fn geometry_and_storage_rejections_leave_the_snapshot_unchanged() {
    let mut device = Device::new(Limits {
        max_width: 8192, max_height: 8192, max_bitmap_bytes: MAX_BITMAP_BYTES,
        max_live_bytes: MAX_LIVE_BYTES, ..limits()
    }).unwrap();
    let before = device.snapshot().unwrap();
    for (format, width, height) in [
        (PixelFormat::Mono1Msb16, 0, 1),
        (PixelFormat::Mono1Msb16, 1, 0),
        (PixelFormat::Rgba8Straight, 8193, 1),
        (PixelFormat::Rgba8Straight, 1, 8193),
        (PixelFormat::Mono1Msb16, u32::MAX, u32::MAX),
        (PixelFormat::Rgba8Straight, u32::MAX, 1),
        (PixelFormat::Rgba8Straight, 8192, 8192),
    ] {
        assert_unsupported(device.create_bitmap(format, width, height));
        assert_eq!(device.snapshot().unwrap(), before);
    }
    let mut tiny = Device::new(Limits {
        max_bitmap_bytes: 1, max_live_bytes: 1, ..limits()
    }).unwrap();
    assert_unsupported(tiny.create_bitmap(PixelFormat::Mono1Msb16, 1, 1));
    assert_eq!(tiny.snapshot().unwrap().bitmap_issuances, 0);
}

#[test]
fn mono_layout_uses_msb_pixels_even_byte_rows_and_real_zero_storage() {
    let mut device = device();
    for (width, stride) in [(1, 2), (8, 2), (9, 2), (16, 2), (17, 4), (31, 4), (32, 4), (33, 6)] {
        let bitmap = device.create_bitmap(PixelFormat::Mono1Msb16, width, 3).unwrap();
        assert_eq!(device.describe_bitmap(&bitmap).unwrap(), BitmapInfo {
            width, height: 3, stride, format: PixelFormat::Mono1Msb16,
        });
        assert_eq!(raw(&device, &bitmap), vec![0; stride * 3]);
        assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, stride * 3);
        device.close_bitmap(&bitmap).unwrap();
        assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
    }
    assert_eq!(device.snapshot().unwrap().bitmap_issuances, 8);
}

#[test]
fn rgba_fill_is_top_down_straight_alpha_and_readback_is_an_independent_copy() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Rgba8Straight, 3, 2);
    assert_eq!(device.describe_bitmap(&bitmap).unwrap().stride, 12);
    assert_eq!(raw(&device, &bitmap), vec![0; 24]);
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![0; 24]);
    device.fill_rect(&selection, rect(1, 0, 2, 1), Pixel::Rgba([9, 17, 33, 7])).unwrap();
    device.fill_rect(&selection, rect(0, 1, 1, 1), Pixel::Rgba([201, 4, 99, 0])).unwrap();
    let mut image = device.readback_rgba(&selection).unwrap();
    assert_eq!((image.width, image.height), (3, 2));
    assert_eq!(image.pixels, vec![
        0, 0, 0, 0, 9, 17, 33, 7, 9, 17, 33, 7,
        201, 4, 99, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    image.pixels.fill(255);
    assert_eq!(pixel(&device.readback_rgba(&selection).unwrap(), 0, 0), [0; 4]);
    assert_eq!(raw(&device, &bitmap)[12..16], [201, 4, 99, 0]);
}

#[test]
fn mono_fills_cross_byte_edges_preserve_padding_and_expand_actual_pixels() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Mono1Msb16, 17, 3);
    let initial = device.readback_rgba(&selection).unwrap();
    assert_eq!(initial.pixels, [0, 0, 0, 255].repeat(51));
    // A private fixture gives unused bits/bytes nonzero values; public drawing
    // must preserve them and readback must ignore them, not mistake them for pixels.
    {
        let mut state = device.lock().unwrap();
        let bytes = state.bitmaps[bitmap.identity.index].pixels.as_mut().unwrap();
        bytes[2] = 0x55;
        bytes[3] = 0xa5;
    }
    device.fill_rect(&selection, rect(0, 0, 17, 1), Pixel::Mono(true)).unwrap();
    device.fill_rect(&selection, rect(7, 1, 3, 1), Pixel::Mono(true)).unwrap();
    device.fill_rect(&selection, rect(8, 1, 1, 1), Pixel::Mono(false)).unwrap();
    device.fill_rect(&selection, rect(16, 2, 1, 1), Pixel::Mono(true)).unwrap();
    assert_eq!(raw(&device, &bitmap), vec![
        0xff, 0xff, 0xd5, 0xa5,
        0x01, 0x40, 0x00, 0x00,
        0x00, 0x00, 0x80, 0x00,
    ]);
    let image = device.readback_rgba(&selection).unwrap();
    for y in 0..3 {
        for x in 0..17 {
            let white = y == 0 || (y == 1 && (x == 7 || x == 9)) || (y == 2 && x == 16);
            assert_eq!(pixel(&image, x, y), if white { [255; 4] } else { [0, 0, 0, 255] });
        }
    }
}

#[test]
fn rectangle_and_format_refusals_preserve_every_pixel_and_selection() {
    for format in [PixelFormat::Mono1Msb16, PixelFormat::Rgba8Straight] {
        let mut device = device();
        let (bitmap, selection) = selected(&mut device, format, 3, 2);
        let paint = match format { PixelFormat::Mono1Msb16 => Pixel::Mono(true),
            PixelFormat::Rgba8Straight => Pixel::Rgba([8, 9, 10, 11]) };
        device.fill_rect(&selection, rect(0, 0, 3, 2), paint).unwrap();
        let before = raw(&device, &bitmap);
        let snapshot = device.snapshot().unwrap();
        let identity = device.selected(&selection).unwrap().unwrap();
        for invalid in [rect(0, 0, 0, 1), rect(0, 0, 1, 0), rect(3, 0, 1, 1),
            rect(0, 2, 1, 1), rect(2, 1, 2, 1), rect(0, 1, 1, 2),
            rect(u32::MAX, 0, 2, 1), rect(0, u32::MAX, 1, 2)]
        {
            assert_unsupported(device.fill_rect(&selection, invalid, paint));
            assert_eq!(raw(&device, &bitmap), before);
            assert_eq!(device.snapshot().unwrap(), snapshot);
            assert!(identity.same_bitmap(&device.selected(&selection).unwrap().unwrap()));
        }
        let mismatch = match format { PixelFormat::Mono1Msb16 => Pixel::Rgba([255; 4]),
            PixelFormat::Rgba8Straight => Pixel::Mono(true) };
        assert_unsupported(device.fill_rect(&selection, rect(0, 0, 3, 2), mismatch));
        assert_eq!(raw(&device, &bitmap), before);
        assert_eq!(device.snapshot().unwrap(), snapshot);
    }
}

#[test]
fn mono_readback_expansion_is_bounded_before_output_allocation() {
    let mut device = Device::new(Limits {
        max_bitmap_bytes: 8, max_live_bytes: 16, ..limits()
    }).unwrap();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Mono1Msb16, 17, 1);
    device.fill_rect(&selection, rect(0, 0, 17, 1), Pixel::Mono(true)).unwrap();
    let before = device.snapshot().unwrap();
    assert_unsupported(device.readback_rgba(&selection));
    assert_eq!(raw(&device, &bitmap), vec![0xff, 0xff, 0x80, 0]);
    assert_eq!(device.snapshot().unwrap(), before);
}

#[test]
fn exclusive_selection_refusal_cannot_displace_the_old_target() {
    let mut device = device();
    let (first, mut one) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let (second, mut two) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    device.fill_rect(&one, rect(0, 0, 1, 1), Pixel::Rgba([1, 2, 3, 4])).unwrap();
    device.fill_rect(&two, rect(0, 0, 1, 1), Pixel::Rgba([5, 6, 7, 8])).unwrap();
    let old_one = device.observe(&first).unwrap();
    let old_two = device.observe(&second).unwrap();
    let snapshot = device.snapshot().unwrap();
    assert_unsupported(device.select(&mut two, &first));
    assert_unsupported(device.select(&mut one, &second));
    assert!(old_one.same_bitmap(&device.selected(&one).unwrap().unwrap()));
    assert!(old_two.same_bitmap(&device.selected(&two).unwrap().unwrap()));
    assert_eq!(device.readback_rgba(&one).unwrap().pixels, vec![1, 2, 3, 4]);
    assert_eq!(device.readback_rgba(&two).unwrap().pixels, vec![5, 6, 7, 8]);
    assert_eq!(device.snapshot().unwrap(), snapshot);
    device.clear(&mut one).unwrap();
    let previous = device.select(&mut two, &first).unwrap().unwrap();
    assert!(previous.same_bitmap(&old_two));
    assert_eq!(device.readback_rgba(&two).unwrap().pixels, vec![1, 2, 3, 4]);
    assert!(device.observation_live(&old_two).unwrap());
    assert_eq!(device.describe_bitmap(&second).unwrap().width, 1);
}

#[test]
fn reselection_is_a_noop_returning_the_actual_previous_observation() {
    let mut device = device();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let before = device.snapshot().unwrap();
    let observation = device.observe(&bitmap).unwrap();
    device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([7, 8, 9, 10])).unwrap();
    let previous = device.select(&mut selection, &bitmap).unwrap().unwrap();
    assert!(previous.same_bitmap(&observation));
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![7, 8, 9, 10]);
    assert_eq!(device.snapshot().unwrap(), before);
}

#[test]
fn selected_close_refuses_then_unselected_close_revokes_exactly_once() {
    let mut device = device();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Mono1Msb16, 1, 1);
    let observation = device.observe(&bitmap).unwrap();
    let before = device.snapshot().unwrap();
    assert_unsupported(device.close_bitmap(&bitmap));
    assert!(device.observation_live(&observation).unwrap());
    assert_eq!(device.snapshot().unwrap(), before);
    device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Mono(true)).unwrap();
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![255; 4]);
    assert!(device.clear(&mut selection).unwrap().unwrap().same_bitmap(&observation));
    device.close_bitmap(&bitmap).unwrap();
    assert!(!device.observation_live(&observation).unwrap());
    assert_unsupported(device.close_bitmap(&bitmap));
    assert_unsupported(device.observe(&bitmap));
    assert_unsupported(device.describe_bitmap(&bitmap));
    assert_unsupported(device.select(&mut selection, &bitmap));
    assert!(device.selected(&selection).unwrap().is_none());
    assert_eq!(device.snapshot().unwrap(), Observation {
        bitmap_issuances: 1, selection_issuances: 1, live_bitmap_bytes: 0,
    });
    drop(bitmap);
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
}

#[test]
fn dropped_caller_owner_revokes_observations_but_retains_selected_pixels() {
    let mut device = device();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 2, 1);
    let observation = device.observe(&bitmap).unwrap();
    let weak_copy = observation.clone();
    drop(bitmap);
    assert!(!device.observation_live(&observation).unwrap());
    assert!(weak_copy.same_bitmap(&device.selected(&selection).unwrap().unwrap()));
    assert_eq!(device.selected_info(&selection).unwrap().unwrap().stride, 8);
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 8);
    device.fill_rect(&selection, rect(0, 0, 2, 1), Pixel::Rgba([20, 30, 40, 50])).unwrap();
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, [20, 30, 40, 50].repeat(2));
    let previous = device.clear(&mut selection).unwrap().unwrap();
    assert!(previous.same_bitmap(&observation));
    assert!(!device.observation_live(&previous).unwrap());
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
    assert!(device.clear(&mut selection).unwrap().is_none());
    assert_unsupported(device.readback_rgba(&selection));
    assert_unsupported(device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Mono(true)));
}

#[test]
fn replacing_orphaned_backing_releases_it_without_closing_the_new_owner() {
    let mut device = device();
    let (old, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let old_observation = device.observe(&old).unwrap();
    let new = device.create_bitmap(PixelFormat::Rgba8Straight, 1, 1).unwrap();
    let new_observation = device.observe(&new).unwrap();
    drop(old);
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 8);
    assert!(device.select(&mut selection, &new).unwrap().unwrap().same_bitmap(&old_observation));
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 4);
    assert!(!device.observation_live(&old_observation).unwrap());
    assert!(device.observation_live(&new_observation).unwrap());
    device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([71; 4])).unwrap();
    assert_eq!(raw(&device, &new), vec![71; 4]);
}

#[test]
fn invalid_replacement_cannot_detach_orphaned_selected_storage() {
    let mut device = device();
    let (orphan, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let observation = device.observe(&orphan).unwrap();
    device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([6, 7, 8, 9])).unwrap();
    drop(orphan);
    let closed = device.create_bitmap(PixelFormat::Rgba8Straight, 1, 1).unwrap();
    device.close_bitmap(&closed).unwrap();
    let (busy, _busy_selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let mut other = Device::new(limits()).unwrap();
    let foreign = other.create_bitmap(PixelFormat::Rgba8Straight, 1, 1).unwrap();
    let before = device.snapshot().unwrap();
    for candidate in [&closed, &busy, &foreign] {
        assert_unsupported(device.select(&mut selection, candidate));
        assert!(device.selected(&selection).unwrap().unwrap().same_bitmap(&observation));
        assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![6, 7, 8, 9]);
        assert_eq!(device.snapshot().unwrap(), before);
    }
    assert!(!device.observation_live(&observation).unwrap());
    device.clear(&mut selection).unwrap();
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 4);
}

#[test]
fn selection_drop_preserves_borrowed_owner_and_releases_its_exclusive_claim() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Mono1Msb16, 2, 1);
    device.fill_rect(&selection, rect(1, 0, 1, 1), Pixel::Mono(true)).unwrap();
    let observation = device.observe(&bitmap).unwrap();
    drop(selection);
    assert!(device.observation_live(&observation).unwrap());
    assert_eq!(device.describe_bitmap(&bitmap).unwrap().stride, 2);
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 2);
    let mut replacement = device.new_selection().unwrap();
    device.select(&mut replacement, &bitmap).unwrap();
    assert_eq!(device.readback_rgba(&replacement).unwrap().pixels, vec![0, 0, 0, 255, 255, 255, 255, 255]);
    device.clear(&mut replacement).unwrap();
    device.close_bitmap(&bitmap).unwrap();
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
}

#[test]
fn selection_drop_releases_orphaned_backing_and_keeps_issuance_history() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Mono1Msb16, 17, 2);
    drop(bitmap);
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 8);
    drop(selection);
    assert_eq!(device.snapshot().unwrap(), Observation {
        bitmap_issuances: 1, selection_issuances: 1, live_bitmap_bytes: 0,
    });
}

#[test]
fn actual_selected_orphan_storage_exhausts_budget_until_detached() {
    let mut device = Device::new(Limits {
        max_bitmap_bytes: 8, max_live_bytes: 8, ..limits()
    }).unwrap();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 2);
    drop(bitmap);
    let before = device.snapshot().unwrap();
    assert_eq!(before.live_bitmap_bytes, 8);
    assert_unsupported(device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1));
    assert_eq!(device.snapshot().unwrap(), before);
    device.fill_rect(&selection, rect(0, 0, 1, 2), Pixel::Rgba([29; 4])).unwrap();
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![29; 8]);
    device.clear(&mut selection).unwrap();
    let next = device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1).unwrap();
    assert_eq!(device.snapshot().unwrap(), Observation {
        bitmap_issuances: 2, selection_issuances: 1, live_bitmap_bytes: 2,
    });
    drop(next);
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
}

#[test]
fn closed_and_dropped_resources_never_replenish_monotonic_history_bounds() {
    let mut device = Device::new(Limits { max_bitmaps: 2, max_selections: 2, ..limits() }).unwrap();
    let first = device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1).unwrap();
    let first_observation = device.observe(&first).unwrap();
    device.close_bitmap(&first).unwrap();
    drop(first);
    let second = device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1).unwrap();
    let second_observation = device.observe(&second).unwrap();
    assert!(!first_observation.same_bitmap(&second_observation));
    assert!(!device.observation_live(&first_observation).unwrap());
    assert!(device.observation_live(&second_observation).unwrap());
    drop(second);
    drop(device.new_selection().unwrap());
    drop(device.new_selection().unwrap());
    let before = device.snapshot().unwrap();
    assert_eq!(before, Observation { bitmap_issuances: 2, selection_issuances: 2, live_bitmap_bytes: 0 });
    assert_unsupported(device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1));
    assert_unsupported(device.new_selection());
    assert_eq!(device.snapshot().unwrap(), before);
    assert!(!device.observation_live(&second_observation).unwrap());
}

#[test]
fn foreign_same_shaped_owners_and_weak_observations_have_no_authority() {
    let mut one = device();
    let mut two = device();
    let (a, mut sa) = selected(&mut one, PixelFormat::Rgba8Straight, 1, 1);
    let (b, mut sb) = selected(&mut two, PixelFormat::Rgba8Straight, 1, 1);
    let oa = one.observe(&a).unwrap();
    let ob = two.observe(&b).unwrap();
    assert_eq!(a.identity.index, b.identity.index);
    assert_eq!(one.describe_bitmap(&a).unwrap(), two.describe_bitmap(&b).unwrap());
    assert!(!oa.same_bitmap(&ob));
    let before_one = one.snapshot().unwrap();
    let before_two = two.snapshot().unwrap();
    assert_unsupported(one.describe_bitmap(&b));
    assert_unsupported(one.observe(&b));
    assert_unsupported(one.observation_live(&ob));
    assert_unsupported(one.close_bitmap(&b));
    assert_unsupported(one.select(&mut sa, &b));
    assert_unsupported(one.select(&mut sb, &a));
    assert_unsupported(one.selected(&sb));
    assert_unsupported(one.selected_info(&sb));
    assert_unsupported(one.clear(&mut sb));
    assert_unsupported(one.fill_rect(&sb, rect(0, 0, 1, 1), Pixel::Rgba([9; 4])));
    assert_unsupported(one.readback_rgba(&sb));
    assert!(oa.same_bitmap(&one.selected(&sa).unwrap().unwrap()));
    assert!(ob.same_bitmap(&two.selected(&sb).unwrap().unwrap()));
    assert_eq!(one.snapshot().unwrap(), before_one);
    assert_eq!(two.snapshot().unwrap(), before_two);
}

#[test]
fn clear_preserves_live_caller_pixels_and_empty_selection_operations_refuse() {
    let mut device = device();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([90, 80, 70, 60])).unwrap();
    device.clear(&mut selection).unwrap();
    let before = device.snapshot().unwrap();
    assert!(device.selected(&selection).unwrap().is_none());
    assert!(device.selected_info(&selection).unwrap().is_none());
    assert_unsupported(device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([255; 4])));
    assert_unsupported(device.readback_rgba(&selection));
    assert_eq!(device.snapshot().unwrap(), before);
    assert_eq!(raw(&device, &bitmap), vec![90, 80, 70, 60]);
    device.select(&mut selection, &bitmap).unwrap();
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![90, 80, 70, 60]);
}

#[test]
fn device_drop_revokes_epoch_while_retained_owners_keep_real_backing_until_drop() {
    let mut one = device();
    let (bitmap, selection) = selected(&mut one, PixelFormat::Rgba8Straight, 2, 1);
    let observation = one.observe(&bitmap).unwrap();
    // Test-only state witness, not an observation upgrade or public owner API.
    let shared = Arc::clone(&one.shared);
    let weak = Arc::downgrade(&shared);
    drop(one);
    assert!(!shared.device_live.load(Ordering::Acquire));
    assert_eq!(shared.state.lock().unwrap().live_bytes().unwrap(), 8);
    assert_unsupported(device().observation_live(&observation));
    drop(bitmap);
    assert_eq!(shared.state.lock().unwrap().live_bytes().unwrap(), 8);
    drop(selection);
    assert_eq!(shared.state.lock().unwrap().live_bytes().unwrap(), 0);
    drop(shared);
    assert_eq!(weak.strong_count(), 0);
    let mut next = device();
    let same_shape = next.create_bitmap(PixelFormat::Rgba8Straight, 2, 1).unwrap();
    assert!(!observation.same_bitmap(&next.observe(&same_shape).unwrap()));
    assert!(observation.same_bitmap(&observation.clone()));
    assert_unsupported(next.observation_live(&observation));
}

#[test]
fn caller_and_selection_drop_on_other_threads_are_safe_and_preserve_borrowed_pixels() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let observation = device.observe(&bitmap).unwrap();
    std::thread::spawn(move || drop(bitmap)).join().unwrap();
    assert!(!device.observation_live(&observation).unwrap());
    device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([12, 34, 56, 78])).unwrap();
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![12, 34, 56, 78]);
    std::thread::spawn(move || drop(selection)).join().unwrap();
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
    let (borrowed, other) = selected(&mut device, PixelFormat::Mono1Msb16, 1, 1);
    device.fill_rect(&other, rect(0, 0, 1, 1), Pixel::Mono(true)).unwrap();
    std::thread::spawn(move || drop(other)).join().unwrap();
    assert_eq!(raw(&device, &borrowed), vec![0x80, 0]);
    assert_eq!(device.describe_bitmap(&borrowed).unwrap().width, 1);
    device.close_bitmap(&borrowed).unwrap();
}

#[test]
fn bitmap_drop_can_race_fill_and_readback_without_losing_selected_backing() {
    let mut device = device();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 2, 2);
    let observation = device.observe(&bitmap).unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let barrier_ref = &barrier;
        let dropping = scope.spawn(move || { barrier_ref.wait(); drop(bitmap); });
        barrier.wait();
        for value in 1..=16 {
            device.fill_rect(&selection, rect(0, 0, 2, 2), Pixel::Rgba([value; 4])).unwrap();
            assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![value; 16]);
            assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 16);
        }
        dropping.join().unwrap();
    });
    assert!(!device.observation_live(&observation).unwrap());
    device.clear(&mut selection).unwrap();
    assert_eq!(device.snapshot().unwrap().live_bitmap_bytes, 0);
}

#[test]
fn moved_device_and_owners_use_the_same_issuing_allocations_on_another_thread() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let before = device.observe(&bitmap).unwrap();
    let (device, bitmap, selection) = std::thread::spawn(move || {
        device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([42, 13, 27, 89])).unwrap();
        (device, bitmap, selection)
    }).join().unwrap();
    assert!(device.observe(&bitmap).unwrap().same_bitmap(&before));
    assert!(device.observation_live(&before).unwrap());
    assert_eq!(device.readback_rgba(&selection).unwrap().pixels, vec![42, 13, 27, 89]);
}

#[test]
fn device_drop_on_another_thread_leaves_only_rust_resource_cleanup() {
    let mut device = device();
    let (bitmap, selection) = selected(&mut device, PixelFormat::Mono1Msb16, 1, 1);
    let weak = Arc::downgrade(&device.shared);
    std::thread::spawn(move || drop(device)).join().unwrap();
    assert_eq!(weak.strong_count(), 2);
    drop(bitmap);
    assert_eq!(weak.strong_count(), 1);
    drop(selection);
    assert_eq!(weak.strong_count(), 0);
}

#[test]
fn poison_is_permanent_refusal_and_drop_only_cleans_without_recovery_success() {
    let mut device = device();
    let (bitmap, mut selection) = selected(&mut device, PixelFormat::Rgba8Straight, 1, 1);
    let observation = device.observe(&bitmap).unwrap();
    let shared = Arc::clone(&device.shared);
    let poisoning = Arc::clone(&shared);
    assert!(std::panic::catch_unwind(move || {
        let _guard = poisoning.state.lock().unwrap();
        panic!("intentional raster poison control");
    }).is_err());
    assert_unavailable(device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1));
    assert_unavailable(device.describe_bitmap(&bitmap));
    assert_unavailable(device.new_selection());
    assert_unavailable(device.select(&mut selection, &bitmap));
    assert_unavailable(device.selected(&selection));
    assert_unavailable(device.selected_info(&selection));
    assert_unavailable(device.clear(&mut selection));
    assert_unavailable(device.close_bitmap(&bitmap));
    assert_unavailable(device.fill_rect(&selection, rect(0, 0, 1, 1), Pixel::Rgba([9; 4])));
    assert_unavailable(device.readback_rgba(&selection));
    assert_unavailable(device.observe(&bitmap));
    assert_unavailable(device.observation_live(&observation));
    assert_unavailable(device.snapshot());
    {
        let state = shared.state.lock().err().unwrap().into_inner();
        assert_eq!(state.live_bytes().unwrap(), 4);
        assert_eq!(state.bitmaps.len(), 1);
        assert_eq!(state.selections.len(), 1);
        assert_eq!(state.bitmaps[0].pixels.as_ref().unwrap(), &vec![0; 4]);
    }
    drop(bitmap);
    drop(selection);
    assert!(shared.state.is_poisoned());
    assert_eq!(shared.state.lock().err().unwrap().into_inner().live_bytes().unwrap(), 0);
    assert_unavailable(device.snapshot());
    assert_unavailable(device.new_selection());
    drop(device);
    assert!(!shared.device_live.load(Ordering::Acquire));
}
