use serde::{Serialize, de::DeserializeOwned};
use sims_render::object_preview::{
    fit::{Definition, Fit},
    surface::{Context, Globals, Image, Plane},
};

fn restore<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn definition() -> Definition {
    Definition {
        group: 0,
        subindex: 0,
        direction: 2,
    }
}

#[test]
fn fitting_preserves_thresholds_signed_centering_and_draw_result() {
    let mut fit = Fit::new(definition(), [120, 120], None);
    // Exactly two pixels of margin fails; three passes. The asymmetric rect
    // makes substituting width/2 or an unsigned division visibly wrong.
    for (zoom, bounds) in [(3, [-20, -70, 96, 40]), (2, [-7, -13, 107, 101])] {
        fit = restore(&fit);
        let request = fit.pending().unwrap();
        assert_eq!(
            (
                request.zoom,
                request.facing,
                request.offset,
                request.bounds_only
            ),
            (zoom, 6, [0, 0], true)
        );
        fit.respond(request, bounds).unwrap();
    }
    let draw = fit.pending().unwrap();
    assert_eq!(
        (draw.zoom, draw.offset, draw.bounds_only),
        (2, [10, 16], false)
    );
    assert!(fit.finish().is_err());
    let before = fit.clone();
    let mut wrong = draw;
    wrong.zoom = 3;
    assert!(fit.respond(wrong, [0; 4]).is_err());
    assert_eq!(fit, before);
    let actual_draw_bounds = [5, 7, 118, 120];
    fit = restore(&fit);
    fit.respond(draw, actual_draw_bounds).unwrap();
    assert_eq!(fit.finish().unwrap().bounds, Some(actual_draw_bounds));
    assert_eq!(fit.finish().unwrap().zoom, 2);
    assert!(fit.respond(draw, actual_draw_bounds).is_err());
}

#[test]
fn fitting_uses_zoom_one_when_nothing_fits_and_preserves_wrapping() {
    let mut fit = Fit::new(definition(), [120, 120], None);
    for zoom in [3, 2, 1] {
        let request = fit.pending().unwrap();
        assert_eq!(request.zoom, zoom);
        fit.respond(request, [0, 0, 127, 125]).unwrap();
    }
    let request = fit.pending().unwrap();
    // -7/2 and -5/2 truncate toward zero, not toward negative infinity.
    assert_eq!(
        (request.zoom, request.offset, request.bounds_only),
        (1, [-3, -2], false)
    );
    let mut empty = Fit::new(definition(), [120, 120], None);
    empty
        .respond(
            empty.pending().unwrap(),
            [i32::MAX, i32::MAX, i32::MIN, i32::MIN],
        )
        .unwrap();
    let request = empty.pending().unwrap();
    assert_eq!((request.zoom, request.offset), (3, [-2_147_483_588; 2]));
    assert!(!request.bounds_only);
}

#[test]
fn fitting_preserves_explicit_zoom_signed_direction_and_multipart_skip() {
    let mut d = definition();
    d.direction = -7;
    for zoom in [0, 1, 2, 3, 8, -1] {
        let mut fit = Fit::new(d, [1, 1], Some(zoom));
        let measure = fit.pending().unwrap();
        assert_eq!((measure.zoom, measure.facing), (zoom, -3));
        fit.respond(measure, [-200, -300, 100, 200]).unwrap();
        assert_eq!(fit.pending().unwrap().zoom, zoom);
        assert!(!fit.pending().unwrap().bounds_only);
        fit.validate().unwrap();
    }
    d.group = 7;
    let skipped = Fit::new(d, [120, 120], Some(3));
    assert_eq!(skipped.pending(), None);
    assert_eq!(skipped.finish().unwrap().zoom, 1);
    assert_eq!(skipped.finish().unwrap().bounds, None);
    d.subindex = -1;
    assert!(Fit::new(d, [120, 120], None).pending().is_some());
}

#[test]
fn malformed_fit_continuations_cannot_change_the_selected_zoom() {
    let fit = Fit::new(definition(), [120, 120], Some(2));
    let mut saved = serde_json::to_value(fit).unwrap();
    saved["stage"]["Measure"] = 3.into();
    let mut corrupt: Fit = serde_json::from_value(saved).unwrap();
    let before = corrupt.clone();
    assert!(corrupt.validate().is_err());
    assert!(corrupt.respond(corrupt.pending().unwrap(), [0; 4]).is_err());
    assert_eq!(corrupt, before);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
enum Call {
    Lock(Plane),
    Buffer(Plane),
    Pitch(Plane),
    Unlock(Plane),
}

#[test]
#[ignore = "requires a freshly executed private native_object_preview.py report"]
fn native_object_preview_helpers_match_original() {
    use serde::Deserialize;
    use sims_render::object_preview::fit::{Output, Request};
    #[derive(Deserialize)]
    struct Row<I, O> {
        input: I,
        original: O,
    }
    #[derive(Deserialize)]
    struct FitInput {
        definition: Definition,
        target: [i32; 2],
        zoom: Option<i32>,
        bounds: [[i32; 4]; 3],
        drawn: [i32; 4],
    }
    #[derive(Deserialize)]
    struct FitResult {
        calls: Vec<Request>,
        output: Output,
    }
    #[derive(Deserialize)]
    struct SurfaceInput {
        bounds: [i32; 4],
        rect: [i32; 4],
        locks: [bool; 2],
        buffers: [u32; 2],
        pitches: [i32; 2],
    }
    #[derive(Deserialize)]
    struct SurfaceResult {
        success: bool,
        after: Globals,
        finished: Globals,
        calls: Vec<Call>,
    }
    #[derive(Deserialize)]
    struct Fixture {
        native_execution: bool,
        source_sha256: String,
        capture_sha256: String,
        fit: Vec<Row<FitInput, FitResult>>,
        surface: Vec<Row<SurfaceInput, SurfaceResult>>,
    }
    let path = std::env::var("SIMS_OBJECT_PREVIEW_NATIVE_FIXTURE").expect("native report path");
    let f: Fixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(
        f.native_execution,
        "prepared inputs are not native execution evidence"
    );
    assert_eq!(
        f.source_sha256,
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        f.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!((f.fit.len(), f.surface.len()), (233, 336));
    for row in f.fit {
        let c = row.input;
        let mut fit = Fit::new(c.definition, c.target, c.zoom);
        let mut calls = Vec::new();
        while let Some(request) = fit.pending() {
            fit = restore(&fit);
            fit.validate().unwrap();
            calls.push(request);
            let reply = if request.bounds_only {
                c.bounds[(3 - request.zoom).clamp(0, 2) as usize]
            } else {
                c.drawn
            };
            fit.respond(request, reply).unwrap();
            assert!(calls.len() <= 4);
        }
        assert_eq!(calls, row.original.calls);
        assert_eq!(fit.finish().unwrap(), row.original.output);
    }
    for row in f.surface {
        let c = row.input;
        let mut image = Surface {
            calls: vec![],
            bounds: c.bounds,
            locks: c.locks,
            buffers: c.buffers,
            pitches: c.pitches,
        };
        let mut context = Context::new(globals());
        assert_eq!(context.begin(&mut image, c.rect), row.original.success);
        assert_eq!(context.globals, row.original.after);
        context = restore(&context);
        context.validate().unwrap();
        context.finish(&mut image);
        assert_eq!(context.globals, row.original.finished);
        assert_eq!(image.calls, row.original.calls);
    }
}

struct Surface {
    calls: Vec<Call>,
    bounds: [i32; 4],
    locks: [bool; 2],
    buffers: [u32; 2],
    pitches: [i32; 2],
}
fn index(p: Plane) -> usize {
    match p {
        Plane::Color => 0,
        Plane::Depth => 1,
    }
}
impl Image for Surface {
    fn bounds(&self) -> [i32; 4] {
        self.bounds
    }
    fn lock(&mut self, p: Plane, f: u32) -> bool {
        assert_eq!(f, 64);
        self.calls.push(Call::Lock(p));
        self.locks[index(p)]
    }
    fn buffer(&mut self, p: Plane) -> u32 {
        self.calls.push(Call::Buffer(p));
        self.buffers[index(p)]
    }
    fn pitch(&mut self, p: Plane) -> i32 {
        self.calls.push(Call::Pitch(p));
        self.pitches[index(p)]
    }
    fn unlock(&mut self, p: Plane, f: u32) {
        assert_eq!(f, 64);
        self.calls.push(Call::Unlock(p));
    }
}
fn surface() -> Surface {
    Surface {
        calls: vec![],
        bounds: [0, 0, 125, 127],
        locks: [true; 2],
        buffers: [0x1000, 0x2000],
        pitches: [250, 256],
    }
}
fn globals() -> Globals {
    Globals {
        color: 10,
        color_pitch: 20,
        depth: 30,
        depth_pitch: 40,
        clip: [1, 2, 3, 4],
        anchor: [5, 6],
    }
}

#[test]
fn surface_binding_rejects_reentry_and_restores_both_targets() {
    use Call::*;
    use Plane::*;
    let mut image = surface();
    let mut context = Context::new(globals());
    assert!(context.begin(&mut image, [2, 3, 122, 123]));
    assert_eq!(
        context.globals,
        Globals {
            color: 0x12f2,
            color_pitch: 250,
            depth: 0x2304,
            depth_pitch: 256,
            clip: [0, 0, 120, 120],
            anchor: [100; 2]
        }
    );
    assert_eq!(
        image.calls,
        [
            Lock(Color),
            Lock(Depth),
            Buffer(Color),
            Pitch(Color),
            Buffer(Depth),
            Pitch(Depth)
        ]
    );
    let bound = context.clone();
    let calls = image.calls.clone();
    assert!(!context.begin(&mut image, [0; 4]));
    assert_eq!(context, bound);
    assert_eq!(image.calls, calls);
    context = restore(&context);
    context.validate().unwrap();
    context.finish(&mut image);
    assert_eq!(&image.calls[6..], [Unlock(Color), Unlock(Depth)]);
    assert_eq!(context.globals, globals());
    assert!(!context.active());
    context.finish(&mut image);
    assert_eq!(image.calls.len(), 8);
}

#[test]
fn surface_failure_retains_native_partial_globals_and_unlock_order() {
    use Call::*;
    use Plane::*;
    for (locks, expected) in [
        ([false, false], vec![Lock(Color), Lock(Depth)]),
        ([false, true], vec![Lock(Color), Lock(Depth), Unlock(Depth)]),
        ([true, false], vec![Lock(Color), Lock(Depth), Unlock(Color)]),
    ] {
        let mut image = surface();
        image.locks = locks;
        let mut context = Context::new(globals());
        assert!(!context.begin(&mut image, [0, 0, 120, 120]));
        assert_eq!(image.calls, expected);
        assert_eq!(
            context.globals,
            Globals {
                color: 0,
                color_pitch: 0,
                depth: 0,
                depth_pitch: 0,
                ..globals()
            }
        );
        let after = context.globals;
        context.finish(&mut image);
        assert_eq!(context.globals, after);
        assert_eq!(image.calls, expected);
    }
    for buffers in [[0, 0x2000], [0x1000, 0], [0, 0]] {
        let mut image = surface();
        image.buffers = buffers;
        let mut context = Context::new(globals());
        assert!(!context.begin(&mut image, [0, 0, 120, 120]));
        assert_eq!(context.globals.color, buffers[0]);
        assert_eq!(context.globals.depth, buffers[1]);
        assert_eq!(
            (context.globals.color_pitch, context.globals.depth_pitch),
            (250, 256)
        );
        assert_eq!(context.globals.clip, globals().clip);
        assert_eq!(context.globals.anchor, globals().anchor);
        assert_eq!(&image.calls[6..], [Unlock(Color), Unlock(Depth)]);
        let before = context.clone();
        context.finish(&mut image);
        assert_eq!(context, before);
    }
}

#[test]
fn surface_clipping_preserves_zero_and_inverted_rectangle_behavior() {
    for (bounds, rect, success) in [
        ([0, 0, 125, 127], [-1, 0, 120, 120], false),
        ([0, 0, 125, 127], [0, 0, 126, 120], false),
        ([0, 0, 125, 127], [125, 0, 125, 127], false),
        // Native 5de95f returns zero for no overlap, then 4e5544 compares
        // coordinates only. A zero target consequently succeeds here.
        ([10, 10, 20, 20], [0, 0, 0, 0], true),
        ([0, 0, 125, 127], [10, 10, 5, 5], true),
    ] {
        let mut image = surface();
        image.bounds = bounds;
        let mut context = Context::new(globals());
        assert_eq!(context.begin(&mut image, rect), success);
        if success {
            context.finish(&mut image);
            assert_eq!(context.globals, globals());
        } else {
            assert!(image.calls.is_empty());
            assert!(!context.active());
        }
    }
}
