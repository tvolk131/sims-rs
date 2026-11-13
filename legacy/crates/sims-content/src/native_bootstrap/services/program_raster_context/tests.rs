//! Authored UNRUN. These controls require the parent's real sibling A module;
//! there is no substitute device, fake pixel backend or native execution here.

use super::*;
use super::program_raster::Limits;

fn limits() -> Limits {
    Limits {
        max_width: 64,
        max_height: 64,
        max_bitmap_bytes: 16 * 1024,
        max_live_bytes: 64 * 1024,
        max_bitmaps: 128,
        max_selections: 128,
    }
}

fn memory() -> Memory {
    Memory::new(4096).unwrap()
}

fn fixture() -> (Memory, Registry, Bitmap) {
    let memory = memory();
    let mut device = Device::new(limits()).unwrap();
    let bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 32).unwrap();
    (memory, registry, bitmap)
}

fn one() -> Rect {
    Rect { x: 0, y: 0, width: 1, height: 1 }
}

fn full() -> Rect {
    Rect { x: 0, y: 0, width: 2, height: 2 }
}

fn refusal<T>(result: Result<T, Stop>, expected: &str) {
    match result {
        Err(Stop::Unsupported(message)) | Err(Stop::Unavailable(message)) => {
            assert!(message.contains(expected), "{message:?} lacks {expected:?}");
        }
        Err(other) => panic!("unexpected refusal: {other:?}"),
        Ok(_) => panic!("operation manufactured success instead of {expected:?}"),
    }
}

// Every operational entry point is checked, including queries and NULL, using
// actual Memory instances and actual creator/foreign threads, not numeric IDs.
fn all_refuse(
    registry: &mut Registry,
    memory: &Memory,
    context: &ContextLease,
    bitmap: &Bitmap,
    observation: &ContextObservation,
    expected: &str,
) {
    refusal(registry.attach_source(memory, bitmap), expected);
    refusal(registry.create_compatible(memory, Some(context)), expected);
    // NULL refuses without touching even a poisoned device/registry lock;
    // heap/thread authentication still runs before the NULL check.
    refusal(registry.create_compatible(memory, None),
        if expected == "poisoned" { "NULL raster source" } else { expected });
    refusal(registry.select_bitmap(memory, context, bitmap), expected);
    refusal(registry.restore_default(memory, context), expected);
    refusal(registry.selected_bitmap(memory, context), expected);
    refusal(registry.bitmap_info(memory, bitmap), expected);
    refusal(registry.set_brush(memory, context, [1, 2, 3, 4]), expected);
    refusal(registry.state(memory, context), expected);
    refusal(registry.fill_rect(memory, context, one()), expected);
    refusal(registry.readback_rgba(memory, context), expected);
    refusal(registry.observe(memory, context), expected);
    refusal(registry.observation_live(memory, observation), expected);
    refusal(registry.close(memory, context), expected);
    refusal(registry.snapshot(memory), expected);
}

#[test]
fn raster_context_owners_and_observations_are_send_sync_without_unsafe() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Device>();
    send_sync::<Bitmap>();
    send_sync::<Selection>();
    send_sync::<BitmapObservation>();
    send_sync::<Registry>();
    send_sync::<ContextLease>();
    send_sync::<ContextObservation>();
    send_sync::<BeginError>();
}

#[test]
fn raster_context_begin_refusal_returns_the_actual_device_and_pixel_owner() {
    let memory = memory();
    for (policy, max) in [
        (Policy::Disabled, 4),
        (Policy::PrivateRasterV1, 0),
        (Policy::PrivateRasterV1, 4097),
        (Policy::PrivateRasterV1, usize::MAX),
    ] {
        let mut device = Device::new(limits()).unwrap();
        let bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
        let mut selection = device.new_selection().unwrap();
        device.select(&mut selection, &bitmap).unwrap();
        device.fill_rect(&selection, full(), Pixel::Rgba([7, 11, 19, 23])).unwrap();
        let observed = device.observe(&bitmap).unwrap();
        let before = device.snapshot().unwrap();
        let error = Registry::new(&memory, device, policy, max).unwrap_err();
        let mut device = error.device;
        assert_eq!(device.snapshot().unwrap(), before);
        assert!(device.observation_live(&observed).unwrap());
        assert_eq!(device.readback_rgba(&selection).unwrap().pixels, [7, 11, 19, 23].repeat(4));
        device.fill_rect(&selection, one(), Pixel::Rgba([31, 37, 41, 43])).unwrap();
        assert_eq!(&device.readback_rgba(&selection).unwrap().pixels[..4], &[31, 37, 41, 43]);
        device.clear(&mut selection).unwrap();
        device.close_bitmap(&bitmap).unwrap();
        assert!(!device.observation_live(&observed).unwrap());
    }
}

#[test]
fn raster_context_null_creation_refuses_before_device_or_history_effects() {
    let (memory, mut registry, bitmap) = fixture();
    let before = registry.snapshot(&memory).unwrap();
    for _ in 0..40 {
        refusal(registry.create_compatible(&memory, None), "NULL raster source");
        assert_eq!(registry.snapshot(&memory).unwrap(), before);
    }
    // A real source still works after more NULL attempts than the B budget.
    let source = registry.attach_source(&memory, &bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &source).unwrap().pixels, [0; 16]);
}

#[test]
fn raster_context_real_attach_brush_fill_and_readback_preserve_straight_alpha() {
    let (memory, mut registry, bitmap) = fixture();
    let actual = registry.inner.lock().unwrap().device.observe(&bitmap).unwrap();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    assert!(registry.selected_bitmap(&memory, &context).unwrap().same_bitmap(&actual));
    let info = registry.bitmap_info(&memory, &bitmap).unwrap();
    assert_eq!((info.width, info.height, info.stride, info.format), (2, 2, 8, PixelFormat::Rgba8Straight));
    assert_eq!(registry.state(&memory, &context).unwrap().brush, [255; 4]);
    registry.set_brush(&memory, &context, [17, 29, 43, 53]).unwrap();
    registry.fill_rect(&memory, &context, full()).unwrap();
    let image = registry.readback_rgba(&memory, &context).unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.pixels, [17, 29, 43, 53].repeat(4));
    registry.set_brush(&memory, &context, [101, 103, 107, 0]).unwrap();
    registry.fill_rect(&memory, &context, one()).unwrap();
    let mut expected = [17, 29, 43, 53].repeat(4);
    expected[..4].copy_from_slice(&[101, 103, 107, 0]);
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, expected);
    // Output copies carry no authority and cannot alter actual backing.
    let mut copy = registry.readback_rgba(&memory, &context).unwrap();
    copy.pixels.fill(255);
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, expected);
}

#[test]
fn raster_context_derived_defaults_are_distinct_real_mono_and_source_independent() {
    let (memory, mut registry, bitmap) = fixture();
    let source = registry.attach_source(&memory, &bitmap).unwrap();
    registry.set_brush(&memory, &source, [13, 17, 19, 23]).unwrap();
    registry.fill_rect(&memory, &source, full()).unwrap();
    let first = registry.create_compatible(&memory, Some(&source)).unwrap();
    let second = registry.create_compatible(&memory, Some(&first)).unwrap();
    let first_bitmap = registry.selected_bitmap(&memory, &first).unwrap();
    let second_bitmap = registry.selected_bitmap(&memory, &second).unwrap();
    assert!(!first_bitmap.same_bitmap(&second_bitmap));
    assert_eq!(registry.state(&memory, &first).unwrap().brush, [255; 4]);
    assert_eq!(registry.state(&memory, &second).unwrap().brush, [255; 4]);
    for context in [&first, &second] {
        let image = registry.readback_rgba(&memory, context).unwrap();
        assert_eq!((image.width, image.height, image.pixels), (1, 1, vec![0, 0, 0, 255]));
        let inner = registry.inner.lock().unwrap();
        let index = registry.context_index(&inner, context).unwrap();
        let resources = inner.records[index].resources.as_ref().unwrap();
        let info = inner.device.describe_bitmap(&resources.default).unwrap();
        assert_eq!((info.width, info.height, info.stride, info.format), (1, 1, 2, PixelFormat::Mono1Msb16));
    }
    assert_eq!(registry.snapshot(&memory).unwrap().device.live_bitmap_bytes, 16 + 3 * 2);
    registry.fill_rect(&memory, &first, one()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &first).unwrap().pixels, [255; 4]);
    assert_eq!(registry.readback_rgba(&memory, &second).unwrap().pixels, [0, 0, 0, 255]);
    let source_observation = registry.observe(&memory, &source).unwrap();
    registry.close(&memory, &source).unwrap();
    drop(source);
    assert!(!registry.observation_live(&memory, &source_observation).unwrap());
    registry.close(&memory, &first).unwrap();
    drop(first);
    // Neither source lease nor source backing is required by the derived one.
    registry.fill_rect(&memory, &second, one()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &second).unwrap().pixels, [255; 4]);
    let borrowed = registry.attach_source(&memory, &bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &borrowed).unwrap().pixels, [13, 17, 19, 23].repeat(4));
}

#[test]
fn raster_context_default_restoration_keeps_real_pixels_and_returns_previous_only() {
    let (memory, mut registry, bitmap) = fixture();
    let source = registry.attach_source(&memory, &bitmap).unwrap();
    let borrowed = registry.selected_bitmap(&memory, &source).unwrap();
    assert!(registry.restore_default(&memory, &source).unwrap().same_bitmap(&borrowed));
    let default = registry.selected_bitmap(&memory, &source).unwrap();
    assert!(!default.same_bitmap(&borrowed));
    registry.fill_rect(&memory, &source, one()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &source).unwrap().pixels, [255; 4]);
    assert!(registry.select_bitmap(&memory, &source, &bitmap).unwrap().same_bitmap(&default));
    let before = registry.snapshot(&memory).unwrap();
    assert!(registry.select_bitmap(&memory, &source, &bitmap).unwrap().same_bitmap(&borrowed));
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
    assert_eq!(registry.readback_rgba(&memory, &source).unwrap().pixels, [0; 16]);
    assert!(registry.restore_default(&memory, &source).unwrap().same_bitmap(&borrowed));
    assert_eq!(registry.readback_rgba(&memory, &source).unwrap().pixels, [255; 4]);
    assert!(registry.restore_default(&memory, &source).unwrap().same_bitmap(&default));
}

#[test]
fn raster_context_mono_brush_and_bad_geometry_refuse_without_pixel_effects() {
    let (memory, mut registry, bitmap) = fixture();
    let source = registry.attach_source(&memory, &bitmap).unwrap();
    let context = registry.create_compatible(&memory, Some(&source)).unwrap();
    let initial = registry.readback_rgba(&memory, &context).unwrap();
    let selected = registry.selected_bitmap(&memory, &context).unwrap();
    for brush in [[1, 1, 1, 255], [255, 255, 255, 254], [0, 0, 0, 0], [255, 0, 0, 255]] {
        registry.set_brush(&memory, &context, brush).unwrap();
        let before = registry.snapshot(&memory).unwrap();
        refusal(registry.fill_rect(&memory, &context, one()), "opaque black or white");
        assert_eq!(registry.readback_rgba(&memory, &context).unwrap(), initial);
        assert_eq!(registry.snapshot(&memory).unwrap(), before);
        assert!(registry.selected_bitmap(&memory, &context).unwrap().same_bitmap(&selected));
    }
    registry.set_brush(&memory, &context, [255; 4]).unwrap();
    for rect in [
        Rect { width: 0, ..one() },
        Rect { height: 0, ..one() },
        Rect { x: 1, ..one() },
        Rect { y: 1, ..one() },
        Rect { x: u32::MAX, width: u32::MAX, ..one() },
    ] {
        let before = registry.snapshot(&memory).unwrap();
        assert!(registry.fill_rect(&memory, &context, rect).is_err());
        assert_eq!(registry.readback_rgba(&memory, &context).unwrap(), initial);
        assert_eq!(registry.snapshot(&memory).unwrap(), before);
    }
    registry.fill_rect(&memory, &context, one()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, [255; 4]);
    registry.set_brush(&memory, &context, [0, 0, 0, 255]).unwrap();
    registry.fill_rect(&memory, &context, one()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap(), initial);
}

#[test]
fn raster_context_borrowed_owner_and_pixels_survive_close_and_repeated_drop() {
    let (memory, mut registry, bitmap) = fixture();
    let bitmap_observation = registry.inner.lock().unwrap().device.observe(&bitmap).unwrap();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    registry.set_brush(&memory, &context, [5, 7, 11, 13]).unwrap();
    registry.fill_rect(&memory, &context, full()).unwrap();
    let observation = registry.observe(&memory, &context).unwrap();
    let cloned = observation.clone();
    registry.close(&memory, &context).unwrap();
    assert!(!registry.observation_live(&memory, &observation).unwrap());
    assert!(!registry.observation_live(&memory, &cloned).unwrap());
    let before = registry.snapshot(&memory).unwrap();
    assert_eq!((before.contexts_issued, before.live_contexts, before.device.live_bitmap_bytes), (1, 0, 16));
    assert!(registry.close(&memory, &context).is_err());
    assert!(registry.create_compatible(&memory, Some(&context)).is_err());
    assert!(registry.observe(&memory, &context).is_err());
    assert!(registry.readback_rgba(&memory, &context).is_err());
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
    drop(context);
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
    assert!(registry.inner.lock().unwrap().device.observation_live(&bitmap_observation).unwrap());
    let later = registry.attach_source(&memory, &bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &later).unwrap().pixels, [5, 7, 11, 13].repeat(4));
    assert!(!registry.observation_live(&memory, &observation).unwrap());
}

#[test]
fn raster_context_lease_drop_detaches_and_releases_default_without_registry_calls() {
    let (memory, mut registry, bitmap) = fixture();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    registry.set_brush(&memory, &context, [2, 3, 5, 7]).unwrap();
    registry.fill_rect(&memory, &context, full()).unwrap();
    let observation = registry.observe(&memory, &context).unwrap();
    drop(context);
    assert!(!registry.observation_live(&memory, &observation).unwrap());
    let after = registry.snapshot(&memory).unwrap();
    assert_eq!((after.contexts_issued, after.live_contexts, after.device.live_bitmap_bytes), (1, 0, 16));
    let later = registry.attach_source(&memory, &bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &later).unwrap().pixels, [2, 3, 5, 7].repeat(4));
}

#[test]
fn raster_context_dropped_bitmap_handle_revokes_weak_owner_but_selected_pixels_survive() {
    let (memory, mut registry, bitmap) = fixture();
    let bitmap_observation = registry.inner.lock().unwrap().device.observe(&bitmap).unwrap();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    drop(bitmap);
    assert!(!registry.inner.lock().unwrap().device.observation_live(&bitmap_observation).unwrap());
    assert_eq!(registry.snapshot(&memory).unwrap().device.live_bitmap_bytes, 18);
    registry.set_brush(&memory, &context, [17, 19, 23, 29]).unwrap();
    registry.fill_rect(&memory, &context, full()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, [17, 19, 23, 29].repeat(4));
    assert!(registry.restore_default(&memory, &context).unwrap().same_bitmap(&bitmap_observation));
    assert_eq!(registry.snapshot(&memory).unwrap().device.live_bitmap_bytes, 2);
    registry.set_brush(&memory, &context, [255; 4]).unwrap();
    registry.fill_rect(&memory, &context, one()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, [255; 4]);
    drop(context);
    assert_eq!(registry.snapshot(&memory).unwrap().device.live_bitmap_bytes, 0);
}

#[test]
fn raster_context_exclusive_selection_and_failed_attach_leave_real_contexts_unchanged() {
    let memory = memory();
    let mut device = Device::new(limits()).unwrap();
    let first_bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let second_bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 8).unwrap();
    let first = registry.attach_source(&memory, &first_bitmap).unwrap();
    let second = registry.attach_source(&memory, &second_bitmap).unwrap();
    registry.set_brush(&memory, &first, [11, 13, 17, 19]).unwrap();
    registry.fill_rect(&memory, &first, full()).unwrap();
    registry.set_brush(&memory, &second, [23, 29, 31, 37]).unwrap();
    registry.fill_rect(&memory, &second, full()).unwrap();
    let first_pixels = registry.readback_rgba(&memory, &first).unwrap();
    let second_pixels = registry.readback_rgba(&memory, &second).unwrap();
    let first_selected = registry.selected_bitmap(&memory, &first).unwrap();
    let second_selected = registry.selected_bitmap(&memory, &second).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    assert!(registry.select_bitmap(&memory, &second, &first_bitmap).is_err());
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
    assert!(registry.attach_source(&memory, &first_bitmap).is_err());
    let after = registry.snapshot(&memory).unwrap();
    assert_eq!(after.live_contexts, before.live_contexts);
    assert_eq!(after.contexts_issued, before.contexts_issued + 1);
    assert_eq!(after.device.bitmap_issuances, before.device.bitmap_issuances + 1);
    assert_eq!(after.device.selection_issuances, before.device.selection_issuances + 1);
    assert_eq!(after.device.live_bitmap_bytes, before.device.live_bitmap_bytes);
    assert!(registry.selected_bitmap(&memory, &first).unwrap().same_bitmap(&first_selected));
    assert!(registry.selected_bitmap(&memory, &second).unwrap().same_bitmap(&second_selected));
    assert_eq!(registry.readback_rgba(&memory, &first).unwrap(), first_pixels);
    assert_eq!(registry.readback_rgba(&memory, &second).unwrap(), second_pixels);
    // The real borrowed owner was not closed, stolen or adopted on failure.
    registry.restore_default(&memory, &first).unwrap();
    assert!(registry.select_bitmap(&memory, &second, &first_bitmap).unwrap().same_bitmap(&second_selected));
    assert_eq!(registry.readback_rgba(&memory, &second).unwrap(), first_pixels);
}

#[test]
fn raster_context_foreign_and_closed_bitmaps_refuse_before_allocations() {
    let (memory, mut registry, bitmap) = fixture();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    let mut foreign_device = Device::new(limits()).unwrap();
    let foreign = foreign_device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let closed = registry.inner.lock().unwrap().device
        .create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    registry.inner.lock().unwrap().device.close_bitmap(&closed).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    let pixels = registry.readback_rgba(&memory, &context).unwrap();
    for rejected in [&foreign, &closed] {
        assert!(registry.bitmap_info(&memory, rejected).is_err());
        assert!(registry.attach_source(&memory, rejected).is_err());
        assert!(registry.select_bitmap(&memory, &context, rejected).is_err());
        assert_eq!(registry.snapshot(&memory).unwrap(), before);
        assert_eq!(registry.readback_rgba(&memory, &context).unwrap(), pixels);
    }
    assert!(foreign_device.describe_bitmap(&foreign).is_ok());
}

#[test]
fn raster_context_already_selected_actual_a_source_is_not_stolen_on_attach_failure() {
    let memory = memory();
    let mut device = Device::new(limits()).unwrap();
    let bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut external_selection = device.new_selection().unwrap();
    device.select(&mut external_selection, &bitmap).unwrap();
    device.fill_rect(&external_selection, full(), Pixel::Rgba([97, 101, 103, 107])).unwrap();
    let actual = device.selected(&external_selection).unwrap().unwrap();
    let mut registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 4).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    assert!(registry.attach_source(&memory, &bitmap).is_err());
    let after = registry.snapshot(&memory).unwrap();
    assert_eq!((after.contexts_issued, after.live_contexts), (1, 0));
    assert_eq!(after.device.live_bitmap_bytes, before.device.live_bitmap_bytes);
    {
        let inner = registry.inner.lock().unwrap();
        assert!(inner.device.selected(&external_selection).unwrap().unwrap().same_bitmap(&actual));
        assert_eq!(inner.device.readback_rgba(&external_selection).unwrap().pixels, [97, 101, 103, 107].repeat(4));
    }
    drop(external_selection);
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &context).unwrap().pixels, [97, 101, 103, 107].repeat(4));
}

#[test]
fn raster_context_foreign_lease_and_weak_observation_never_authenticate_equal_shapes() {
    let (memory, mut first, first_bitmap) = fixture();
    let mut second_device = Device::new(limits()).unwrap();
    let second_bitmap = second_device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut second = Registry::new(&memory, second_device, Policy::PrivateRasterV1, 32).unwrap();
    let first_context = first.attach_source(&memory, &first_bitmap).unwrap();
    let second_context = second.attach_source(&memory, &second_bitmap).unwrap();
    let observation = first.observe(&memory, &first_context).unwrap();
    let before = second.snapshot(&memory).unwrap();
    assert!(second.create_compatible(&memory, Some(&first_context)).is_err());
    assert!(second.select_bitmap(&memory, &first_context, &second_bitmap).is_err());
    assert!(second.restore_default(&memory, &first_context).is_err());
    assert!(second.selected_bitmap(&memory, &first_context).is_err());
    assert!(second.set_brush(&memory, &first_context, [1; 4]).is_err());
    assert!(second.state(&memory, &first_context).is_err());
    assert!(second.fill_rect(&memory, &first_context, one()).is_err());
    assert!(second.readback_rgba(&memory, &first_context).is_err());
    assert!(second.observe(&memory, &first_context).is_err());
    assert!(second.observation_live(&memory, &observation).is_err());
    assert!(second.close(&memory, &first_context).is_err());
    assert_eq!(second.snapshot(&memory).unwrap(), before);
    assert_eq!(second.readback_rgba(&memory, &second_context).unwrap().pixels, [0; 16]);
    assert!(first.observation_live(&memory, &observation).unwrap());
}

#[test]
fn raster_context_heap_swap_and_same_address_equal_allocations_refuse_before_effects() {
    let (mut memory, mut registry, bitmap) = fixture();
    let actual = memory.allocate_owner(16).unwrap();
    let mut other = self::memory();
    let equal_address = other.allocate_owner(16).unwrap();
    assert_eq!(actual.token, equal_address.token);
    assert_eq!(actual.data, equal_address.data);
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    let observation = registry.observe(&memory, &context).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    all_refuse(&mut registry, &other, &context, &bitmap, &observation, "another heap");
    // A new actual heap occupies the same stack variable; pointer address and
    // repeated logical allocation addresses confer no registry authority.
    std::mem::swap(&mut memory, &mut other);
    all_refuse(&mut registry, &memory, &context, &bitmap, &observation, "another heap");
    assert_eq!(registry.snapshot(&other).unwrap(), before);
    registry.set_brush(&other, &context, [41, 43, 47, 53]).unwrap();
    registry.fill_rect(&other, &context, full()).unwrap();
    assert_eq!(registry.readback_rgba(&other, &context).unwrap().pixels, [41, 43, 47, 53].repeat(4));
}

#[test]
fn raster_context_actual_foreign_thread_refuses_every_entry_and_heap_is_checked_first() {
    let (memory, mut registry, bitmap) = fixture();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    let observation = registry.observe(&memory, &context).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    let foreign_heap = self::memory();
    thread::scope(|scope| {
        scope.spawn(|| {
            all_refuse(&mut registry, &memory, &context, &bitmap, &observation, "creator thread");
            all_refuse(&mut registry, &foreign_heap, &context, &bitmap, &observation, "another heap");
        }).join().unwrap();
    });
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
    registry.fill_rect(&memory, &context, one()).unwrap();
    assert_eq!(&registry.readback_rgba(&memory, &context).unwrap().pixels[..4], &[255; 4]);
}

#[test]
fn raster_context_actual_foreign_thread_can_drop_lease_and_preserve_borrowed_pixels() {
    let (memory, mut registry, bitmap) = fixture();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    registry.set_brush(&memory, &context, [59, 61, 67, 71]).unwrap();
    registry.fill_rect(&memory, &context, full()).unwrap();
    let observation = registry.observe(&memory, &context).unwrap();
    thread::scope(|scope| {
        scope.spawn(move || drop(context)).join().unwrap();
    });
    assert!(!registry.observation_live(&memory, &observation).unwrap());
    let after = registry.snapshot(&memory).unwrap();
    assert_eq!((after.live_contexts, after.device.live_bitmap_bytes), (0, 16));
    let next = registry.attach_source(&memory, &bitmap).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &next).unwrap().pixels, [59, 61, 67, 71].repeat(4));
}

#[test]
fn raster_context_registry_drop_revokes_issuer_and_leases_do_not_keep_it_alive() {
    let (memory, mut registry, bitmap) = fixture();
    let source = registry.attach_source(&memory, &bitmap).unwrap();
    let derived = registry.create_compatible(&memory, Some(&source)).unwrap();
    let observation = registry.observe(&memory, &derived).unwrap();
    let weak_registry = Arc::downgrade(&registry.inner);
    thread::scope(|scope| {
        scope.spawn(move || drop(registry)).join().unwrap();
    });
    assert_eq!(weak_registry.strong_count(), 0);
    assert_eq!(observation.issuer.strong_count(), 0);
    let mut replacement = Registry::new(&memory, Device::new(limits()).unwrap(), Policy::PrivateRasterV1, 32).unwrap();
    let before = replacement.snapshot(&memory).unwrap();
    assert!(replacement.create_compatible(&memory, Some(&source)).is_err());
    assert!(replacement.create_compatible(&memory, Some(&derived)).is_err());
    assert!(replacement.observation_live(&memory, &observation).is_err());
    assert!(replacement.bitmap_info(&memory, &bitmap).is_err());
    assert_eq!(replacement.snapshot(&memory).unwrap(), before);
    drop(source);
    drop(derived);
    assert_eq!(weak_registry.strong_count(), 0);
}

#[test]
fn raster_context_lifetime_budget_never_recycles_closed_dropped_or_failed_slots() {
    let memory = memory();
    let mut device = Device::new(limits()).unwrap();
    let bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 3).unwrap();
    let first = registry.attach_source(&memory, &bitmap).unwrap();
    let old = registry.observe(&memory, &first).unwrap();
    registry.close(&memory, &first).unwrap();
    let second = registry.attach_source(&memory, &bitmap).unwrap();
    let later = registry.observe(&memory, &second).unwrap();
    assert!(!Weak::ptr_eq(&old.identity, &later.identity));
    // Failure after real A allocation consumes an honest history slot too.
    assert!(registry.attach_source(&memory, &bitmap).is_err());
    drop(second);
    assert!(!registry.observation_live(&memory, &old).unwrap());
    assert!(!registry.observation_live(&memory, &later).unwrap());
    let before = registry.snapshot(&memory).unwrap();
    assert_eq!((before.contexts_issued, before.live_contexts), (3, 0));
    refusal(registry.attach_source(&memory, &bitmap), "history is exhausted");
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
    assert!(registry.create_compatible(&memory, Some(&first)).is_err());
    drop(first);
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
}

#[test]
fn raster_context_partial_a_bitmap_failure_has_no_fake_lease_or_live_default() {
    let memory = memory();
    let mut device = Device::new(Limits { max_bitmaps: 1, ..limits() }).unwrap();
    let bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 2).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    assert!(registry.attach_source(&memory, &bitmap).is_err());
    let after = registry.snapshot(&memory).unwrap();
    assert_eq!((after.contexts_issued, after.live_contexts), (1, 0));
    assert_eq!(after.device.bitmap_issuances, before.device.bitmap_issuances);
    assert_eq!(after.device.selection_issuances, before.device.selection_issuances + 1);
    assert_eq!(after.device.live_bitmap_bytes, 16);
    // The unsuccessful context detached every unpublished selection.
    let mut inner = registry.inner.lock().unwrap();
    let mut actual_selection = inner.device.new_selection().unwrap();
    inner.device.select(&mut actual_selection, &bitmap).unwrap();
    inner.device.fill_rect(&actual_selection, full(), Pixel::Rgba([73, 79, 83, 89])).unwrap();
    assert_eq!(inner.device.readback_rgba(&actual_selection).unwrap().pixels, [73, 79, 83, 89].repeat(4));
}

#[test]
fn raster_context_real_a_selection_exhaustion_and_live_byte_limits_never_fake_success() {
    let memory = memory();
    let mut device = Device::new(Limits { max_selections: 1, ..limits() }).unwrap();
    let bitmap = device.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 8).unwrap();
    let source = registry.attach_source(&memory, &bitmap).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    assert!(registry.create_compatible(&memory, Some(&source)).is_err());
    let after = registry.snapshot(&memory).unwrap();
    assert_eq!((after.contexts_issued, after.live_contexts), (2, 1));
    assert_eq!(after.device, before.device);
    registry.fill_rect(&memory, &source, full()).unwrap();
    assert_eq!(registry.readback_rgba(&memory, &source).unwrap().pixels, [255; 16]);

    let mut bounded = Device::new(Limits { max_bitmap_bytes: 16, max_live_bytes: 16, ..limits() }).unwrap();
    let full_budget = bounded.create_bitmap(PixelFormat::Rgba8Straight, 2, 2).unwrap();
    let mut bounded = Registry::new(&memory, bounded, Policy::PrivateRasterV1, 2).unwrap();
    assert!(bounded.attach_source(&memory, &full_budget).is_err());
    let after = bounded.snapshot(&memory).unwrap();
    assert_eq!((after.contexts_issued, after.live_contexts), (1, 0));
    assert_eq!(after.device.live_bitmap_bytes, 16);
    assert!(bounded.bitmap_info(&memory, &full_budget).is_ok());
}

#[test]
fn raster_context_actual_mono_readback_bound_is_refusal_not_empty_image() {
    let memory = memory();
    let mut device = Device::new(Limits { max_bitmap_bytes: 2, max_live_bytes: 8, ..limits() }).unwrap();
    let bitmap = device.create_bitmap(PixelFormat::Mono1Msb16, 1, 1).unwrap();
    let mut registry = Registry::new(&memory, device, Policy::PrivateRasterV1, 4).unwrap();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    let before = registry.snapshot(&memory).unwrap();
    assert!(registry.readback_rgba(&memory, &context).is_err());
    registry.fill_rect(&memory, &context, one()).unwrap();
    assert!(registry.readback_rgba(&memory, &context).is_err());
    assert_eq!(registry.snapshot(&memory).unwrap(), before);
}

#[test]
fn raster_context_poison_is_permanent_refusal_but_cross_thread_drop_still_cleans() {
    let (memory, mut registry, bitmap) = fixture();
    let context = registry.attach_source(&memory, &bitmap).unwrap();
    let observation = registry.observe(&memory, &context).unwrap();
    // Poison the actual B synchronization primitive; no mock fault callback.
    thread::scope(|scope| {
        let inner = &registry.inner;
        assert!(scope.spawn(|| {
            let _guard = inner.lock().unwrap();
            panic!("authored registry poison control");
        }).join().is_err());
    });
    all_refuse(&mut registry, &memory, &context, &bitmap, &observation, "poisoned");
    thread::scope(|scope| {
        scope.spawn(move || drop(context)).join().unwrap();
    });
    // Teardown alone may recover a guard; it must not clear/adopt poison.
    assert!(registry.inner.is_poisoned());
    {
        let inner = match registry.inner.lock() {
            Ok(_) => panic!("poison was improperly cleared"),
            Err(poison) => poison.into_inner(),
        };
        assert!(inner.records.iter().all(|r| r.resources.is_none()));
        assert_eq!(inner.device.snapshot().unwrap().live_bitmap_bytes, 16);
    }
    refusal(registry.snapshot(&memory), "poisoned");
    let weak_registry = Arc::downgrade(&registry.inner);
    drop(registry);
    assert_eq!(weak_registry.strong_count(), 0);
}
