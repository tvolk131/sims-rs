use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_formats::bitmap::Image;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::loading::{
    LOAD_LOOP_ASSET, LOAD_MUSIC_MODE_ID, LoadingAudioCue, LoadingKind, LoadingLayout,
    LoadingScreen, LoadingState, LoadingStrings, art_id, load_art,
};
use std::path::PathBuf;

fn sims_install() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.width > 0
        && a.height > 0
        && b.width > 0
        && b.height > 0
        && a.x < b.x + b.width
        && b.x < a.x + a.width
        && a.y < b.y + b.height
        && b.y < a.y + a.height
}

fn synthetic_art() -> Art {
    let mut art = Art::default();
    let img800 = Image {
        width: 800,
        height: 600,
        rgba: [40, 80, 140, 255].repeat(800 * 600),
    };
    let img1024 = Image {
        width: 1024,
        height: 768,
        rgba: [60, 100, 180, 255].repeat(1024 * 768),
    };
    art.insert(art_id::STARTUP_SPLASH, img800.clone());
    art.insert(art_id::DOWNTOWN_800, img800);
    art.insert(art_id::DOWNTOWN_1024, img1024);
    art
}

#[test]
fn audio_lifecycle_hook_starts_and_stops_load_loop_idempotently() {
    let mut screen = LoadingScreen::new();
    assert!(!screen.is_audio_active());

    let start = screen.begin(0);
    assert_eq!(
        start,
        Some(LoadingAudioCue::StartLoadLoop {
            mode_id: LOAD_MUSIC_MODE_ID,
            asset_path: LOAD_LOOP_ASSET,
        })
    );
    assert!(screen.is_audio_active());

    // Calling begin again while already active does not duplicate the cue.
    assert_eq!(screen.begin(2), None);
    assert_eq!(screen.caption_index(), 2);

    assert_eq!(screen.finish(), Some(LoadingAudioCue::StopLoadLoop));
    assert!(!screen.is_audio_active());
    assert_eq!(screen.finish(), None);
}

#[test]
fn rotating_captions_advance_deterministically_and_clamps_progress() {
    let mut screen = LoadingScreen::new();
    screen.set_caption_interval_ms(1_000);
    let _ = screen.begin(0);

    let strings = LoadingStrings {
        progress_captions: vec![
            "Stage Alpha".to_owned(),
            "Stage Beta".to_owned(),
            "Stage Gamma".to_owned(),
        ],
        celebrity_captions: vec!["Studio Stage A".to_owned(), "Studio Stage B".to_owned()],
    };

    let state = LoadingState {
        kind: LoadingKind::Startup,
        progress: f32::NAN,
        destination_label: None,
        caption_override: None,
    };
    assert_eq!(state.clamped_progress(), 0.0);
    assert_eq!(state.percent(), 0);
    assert_eq!(screen.active_caption(&state, &strings), "Stage Alpha");

    screen.tick(1_000);
    assert_eq!(screen.active_caption(&state, &strings), "Stage Beta");

    screen.tick(2_000);
    assert_eq!(screen.active_caption(&state, &strings), "Stage Alpha");

    // StudioTown uses celebrity_captions pool when non-empty.
    let studio_state = LoadingState {
        kind: LoadingKind::StudioTown { fan_bus: false },
        progress: 1.5,
        destination_label: Some("Lot 81".to_owned()),
        caption_override: None,
    };
    assert_eq!(studio_state.clamped_progress(), 1.0);
    assert_eq!(studio_state.percent(), 100);
    assert_eq!(
        screen.active_caption(&studio_state, &strings),
        "Studio Stage B"
    );

    // Fallback percentage when string pool is empty.
    let empty_strings = LoadingStrings::default();
    let half_state = LoadingState {
        kind: LoadingKind::Lot,
        progress: 0.42,
        ..LoadingState::default()
    };
    assert_eq!(screen.active_caption(&half_state, &empty_strings), "42%");
}

#[test]
fn layout_centers_above_800x600_and_compacts_cleanly_below() {
    let art = synthetic_art();

    // 1280x800 picks 1024x768 Downtown art and centers it cleanly.
    let wide = LoadingLayout::compute(1280, 800, LoadingKind::Downtown, &art);
    assert!(!wide.compact);
    assert_eq!(wide.canvas_rect, Rect::new(128, 16, 1024, 768));
    assert!(!rects_overlap(wide.header_rect, wide.progress_rect));
    assert!(!rects_overlap(wide.progress_rect, wide.caption_rect));

    // 800x600 exact native centering.
    let native = LoadingLayout::compute(800, 600, LoadingKind::Downtown, &art);
    assert!(!native.compact);
    assert_eq!(native.canvas_rect, Rect::new(0, 0, 800, 600));
    assert!(!rects_overlap(native.progress_rect, native.caption_rect));

    // Compact viewports (< 800x600): 640x480 and 320x240 stay strictly inside bounds.
    for (w, h) in [(640u32, 480u32), (320, 240)] {
        let compact = LoadingLayout::compute(w, h, LoadingKind::Downtown, &art);
        assert!(compact.compact);
        assert_eq!(compact.canvas_rect, Rect::new(0, 0, w as i32, h as i32));
        for r in [
            compact.header_rect,
            compact.progress_rect,
            compact.caption_rect,
        ] {
            assert!(r.x >= 0 && r.y >= 0);
            assert!(r.x + r.width <= w as i32);
            assert!(r.y + r.height <= h as i32);
        }
        assert!(!rects_overlap(compact.header_rect, compact.progress_rect));
        assert!(!rects_overlap(compact.progress_rect, compact.caption_rect));
    }
}

#[test]
fn draw_animates_progress_bar_across_progress_and_time() {
    let art = synthetic_art();
    let strings = LoadingStrings::default();
    let mut screen = LoadingScreen::new();
    let _ = screen.begin(0);

    let mut f0 = Frame::new(800, 600, [0, 0, 0]);
    let s0 = LoadingState {
        kind: LoadingKind::Downtown,
        progress: 0.1,
        ..LoadingState::default()
    };
    screen.draw(&mut f0, &s0, &art, &strings);

    screen.tick(320);
    let mut f1 = Frame::new(800, 600, [0, 0, 0]);
    let s1 = LoadingState {
        kind: LoadingKind::Downtown,
        progress: 0.75,
        ..LoadingState::default()
    };
    screen.draw(&mut f1, &s1, &art, &strings);

    assert_ne!(f0.rgb(), f1.rgb());
}

#[test]
fn complete_collection_loads_splash_art_and_localized_captions() {
    let Some(root) = sims_install() else {
        return;
    };
    let install = Install::detect(&root).expect("open Complete Collection install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, US_ENGLISH).expect("load loading screen art");
    let strings = LoadingStrings::load(&install, US_ENGLISH).expect("load UIText.iff strings");

    assert!(
        art.missing.is_empty(),
        "unexpected missing loading art IDs: {:?}",
        art.missing
    );
    assert!(
        strings.progress_captions.len() >= 15,
        "expected >= 15 splashprogress captions in STR# 155, got {}",
        strings.progress_captions.len()
    );
    assert!(
        !strings.celebrity_captions.is_empty(),
        "expected celebrity loading captions in STR# 271"
    );

    let mut screen = LoadingScreen::new();
    let _ = screen.begin(0);
    for (kind, (w, h)) in [
        (LoadingKind::Startup, (800, 600)),
        (LoadingKind::Downtown, (1024, 768)),
        (LoadingKind::Vacation { alternate: true }, (800, 600)),
        (LoadingKind::StudioTown { fan_bus: true }, (1024, 768)),
        (LoadingKind::MagicTown { via_hole: true }, (640, 480)),
    ] {
        let mut frame = Frame::new(w, h, [0, 0, 0]);
        let state = LoadingState {
            kind,
            progress: 0.5,
            destination_label: None,
            caption_override: None,
        };
        let layout = screen.draw(&mut frame, &state, &art, &strings);
        assert!(layout.progress_rect.width > 0);
        assert!(layout.caption_rect.width > 0);
        assert!(!screen.active_caption(&state, &strings).is_empty());
    }
}
