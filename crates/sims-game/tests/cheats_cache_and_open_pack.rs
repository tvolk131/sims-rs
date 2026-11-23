//! Headless E2E verification for Batch 13 Track 1:
//! - Zero-`SIMS_INSTALL` standalone clean-room `open-pack/` boot, rendering,
//!   interaction pie menu, camera rotations/zooms, raised-terrain Sim elevation
//!   & depth projection, Buy Mode + live cheats, synthesized WAV audio,
//!   two-pack isolated save slots & custom archive names, and CLI
//!   `--open-pack --png` / automatic standalone fallback (with decoded RGB check).
//! - `SIMS_INSTALL`-gated Complete Collection verification for native live
//!   cheats (`move_objects`, `rosebud`/`klapaucius`, `set_hour` preserving
//!   Buy/Build undo history), warm `NeighborhoodShell::enter_lot` with
//!   `VmInstallCache` (full state/render/audio/save parity + 5-run median
//!   timing), native REDB v4 `StoredNeighborhood.family_surnames` driven through
//!   `NeighborhoodShell` (zero `.caf-names.tsv` writes + one-time legacy
//!   migration that never resurrects deleted families), and `AutosaveStore`
//!   SHA-256 fast path.

use sims_audio::SoundEvent;
use sims_game::autosave_worker::{AutosaveWorker, validate_save_payload_cached_with_sha256};
use sims_game::game::Game;
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::open_pack::{
    OpenSimFacing, discover_open_pack_dir, open_pack_save_slot, open_pack_sim_facing,
    project_open_pack_sim,
};
use sims_game::sound::{Input, Mixer};
use sims_game::view::Move;
use sims_render::depth::Depth;
use sims_render::{Camera, Lot as RenderLot, Zoom};
use sims_sim::autosave::{AutosaveStore, SaveError, SaveJob, SavePayload, SaveSlot};
use sims_ui::command::Mode;
use sims_vm::boot::Frames;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

struct TempTestDir(PathBuf);

impl TempTestDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sims-game-b13-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_pack_path() -> PathBuf {
    discover_open_pack_dir().expect("workspace open-pack/ directory must be discoverable")
}

fn submit_cheat(game: &mut Game, line: &str) {
    if !game.cheat_console_open() {
        game.toggle_cheat_console();
    }
    for ch in line.chars() {
        game.cheat_console_char(ch);
    }
    game.cheat_console_submit()
        .unwrap_or_else(|e| panic!("cheat {line:?} parse failed: {e}"));
}

#[allow(clippy::disallowed_methods)]
fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    let mut entries: Vec<_> = std::fs::read_dir(src)
        .expect("read src dir")
        .map(|e| e.expect("dir entry"))
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let ty = entry.file_type().expect("file type");
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else if ty.is_file() {
            std::fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

/// Decodes uncompressed 8-bit RGB PNG bytes written by `sims_render::png::encode`
/// into `(width, height, rgb_bytes)`.
fn decode_stored_rgb_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG signature");
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..pos + 8 + len];
        pos += 12 + len;
        if kind == b"IHDR" {
            width = u32::from_be_bytes(data[0..4].try_into().unwrap());
            height = u32::from_be_bytes(data[4..8].try_into().unwrap());
            assert_eq!(&data[8..13], &[8, 2, 0, 0, 0], "expected 8-bit RGB PNG");
        } else if kind == b"IDAT" {
            idat.extend_from_slice(data);
        } else if kind == b"IEND" {
            break;
        }
    }
    assert!(width > 0 && height > 0, "valid PNG dimensions");
    assert!(idat.len() >= 6, "valid zlib stream");
    let mut zpos = 2usize; // skip 2-byte zlib header
    let mut raw = Vec::new();
    while zpos + 5 <= idat.len() - 4 {
        let header = idat[zpos];
        assert_eq!(header & 0x06, 0, "expected stored deflate block");
        let blen = u16::from_le_bytes([idat[zpos + 1], idat[zpos + 2]]) as usize;
        let nlen = u16::from_le_bytes([idat[zpos + 3], idat[zpos + 4]]);
        assert_eq!(blen as u16, !nlen, "valid stored block length");
        zpos += 5;
        raw.extend_from_slice(&idat[zpos..zpos + blen]);
        zpos += blen;
        if header & 1 != 0 {
            break;
        }
    }
    let row_bytes = (width as usize) * 3 + 1;
    assert_eq!(raw.len(), row_bytes * (height as usize));
    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for row in raw.chunks_exact(row_bytes) {
        assert_eq!(row[0], 0, "expected filter type 0");
        rgb.extend_from_slice(&row[1..]);
    }
    (width, height, rgb)
}

#[test]
fn open_pack_boots_renders_steps_interacts_cheats_and_plays_wavs() {
    let pack_dir = open_pack_path();
    let mut game = Game::open_open_pack(&pack_dir, &[], false, Frames::default())
        .expect("standalone open-pack/ must boot cleanly without SIMS_INSTALL");

    assert!(game.is_open_pack());
    assert!(
        game.stopped().is_none(),
        "open-pack lot must not stop on load: {:?}",
        game.stopped()
    );
    assert!(
        game.errors().is_empty(),
        "open-pack lot load must have 0 errors: {:?}",
        game.errors()
    );

    // Verify all starter objects and both starter Sims exist in the booted world.
    let world = game.lot().world();
    let sim_ids: Vec<u16> = world
        .objects
        .iter()
        .filter_map(|(&id, obj)| obj.person.is_some().then_some(id))
        .collect();
    assert_eq!(sim_ids.len(), 2, "open-pack starter house must boot 2 Sims");
    assert!(
        world.objects.len() >= 18,
        "expected starter cottage objects + Sims, got {}",
        world.objects.len()
    );
    assert!(
        world.selected().is_some(),
        "a starter Sim must be selected on boot"
    );

    // Render into 800x600 Canvas and verify non-blank lot viewport + HUD + visible objects & Sims.
    game.resize([800, 600]);
    game.render();
    let canvas = game.canvas();
    assert_eq!(canvas.width, 800);
    assert_eq!(canvas.height, 600);

    let mut lot_colors = BTreeSet::new();
    for y in 0..500usize {
        for x in (0..800usize).step_by(4) {
            lot_colors.insert(canvas.pixel_rgb(x, y));
        }
    }
    assert!(
        lot_colors.len() >= 16,
        "rendered open-pack lot viewport must be visually rich (got {} distinct colors)",
        lot_colors.len()
    );

    let mut hud_colors = BTreeSet::new();
    for y in 500..600usize {
        for x in (0..800usize).step_by(4) {
            hud_colors.insert(canvas.pixel_rgb(x, y));
        }
    }
    assert!(
        hud_colors.len() >= 6,
        "clean-room procedural HUD strip must render legibly (got {} distinct colors)",
        hud_colors.len()
    );

    // Verify BOTH starter Sims render visibly across all 4 camera rotations and all 3 zooms.
    for rot in 0..4 {
        game.render();
        for &sim_id in &sim_ids {
            assert!(
                game.canvas().drawn(sim_id).is_some(),
                "starter Sim {sim_id} must render visibly at rotation {rot}"
            );
        }
        assert!(game.apply(Move::Rotate(true)));
    }
    // Zoom in to Near and out to Far, confirming both starter Sims still draw cleanly.
    let _ = game.apply(Move::ZoomIn);
    game.render();
    assert!(sim_ids.iter().any(|&id| game.canvas().drawn(id).is_some()));
    let _ = game.apply(Move::ZoomOut);
    let _ = game.apply(Move::ZoomOut);
    game.render();
    for &sim_id in &sim_ids {
        assert!(
            game.canvas().drawn(sim_id).is_some(),
            "starter Sim {sim_id} must render visibly at Far zoom"
        );
    }
    let _ = game.apply(Move::ZoomIn);
    game.render();

    // Verify camera-relative Sim facing matches projected VM movement direction
    // across all 8 VM facings x 4 camera rotations (32 combinations).
    let vm_steps: [[i32; 2]; 8] = [
        [-1, 0],
        [-1, 1],
        [0, 1],
        [1, 1],
        [1, 0],
        [1, -1],
        [0, -1],
        [-1, -1],
    ];
    for rotation in 0u8..4 {
        let cam = Camera {
            zoom: Zoom::Medium,
            rotation,
            level: 1,
            origin: [400, 120],
            walls: sims_render::Walls::Up,
            focus: sims_render::Focus::default(),
        };
        for (dir, &[dx, dy]) in vm_steps.iter().enumerate() {
            let p0 = cam.project_fine(cam.rotate_fine([32 * 16, 32 * 16]), 1, 0);
            let p1 = cam.project_fine(
                cam.rotate_fine([32 * 16 + dx * 16, 32 * 16 + dy * 16]),
                1,
                0,
            );
            let sdx = (p1[0] - p0[0]).signum();
            let sdy = (p1[1] - p0[1]).signum();
            let facing = open_pack_sim_facing(&cam, dir as i16);
            assert_eq!(
                facing.screen_x_sign(),
                sdx,
                "facing {dir} at rotation {rotation} must match projected horizontal sign"
            );
            assert_eq!(
                facing.is_away(),
                sdy < 0,
                "facing {dir} at rotation {rotation} must match projected depth direction"
            );
        }
    }
    // Explicit regression check for facing 0 ([-1, 0]) at rotation 1 -> moves screen-right & away.
    let rot1_cam = Camera {
        zoom: Zoom::Medium,
        rotation: 1,
        level: 1,
        origin: [400, 120],
        walls: sims_render::Walls::Up,
        focus: sims_render::Focus::default(),
    };
    assert_eq!(open_pack_sim_facing(&rot1_cam, 0), OpenSimFacing::BackRight);

    // Verify raised-terrain projection & depth anchor at all 3 zooms (1 step = 16 near-zoom px).
    let tile_x = 16usize;
    let tile_y = 16usize;
    let fine = [(tile_x as i32) * 16 + 8, (tile_y as i32) * 16 + 8, 1];
    let mut flat_lot = RenderLot::from_world(game.lot().world()).expect("flat lot");
    flat_lot.heights[RenderLot::cell(tile_x, tile_y)] = [0, 0, 0, 0];
    let mut raised_lot = RenderLot::from_world(game.lot().world()).expect("raised lot");
    raised_lot.heights[RenderLot::cell(tile_x, tile_y)] = [2, 2, 2, 2];
    for (zoom, expected_dy) in [(Zoom::Far, 8i32), (Zoom::Medium, 16), (Zoom::Near, 32)] {
        let cam = Camera {
            zoom,
            rotation: 0,
            level: 1,
            origin: [400, 120],
            walls: sims_render::Walls::Up,
            focus: sims_render::Focus::default(),
        };
        let (flat_screen, _) = project_open_pack_sim(&cam, &flat_lot, fine);
        let (raised_screen, raised_anchor) = project_open_pack_sim(&cam, &raised_lot, fine);
        assert_eq!(
            flat_screen[1] - raised_screen[1],
            expected_dy,
            "2-step raised tile must lift Sim by {expected_dy} px at {zoom:?}"
        );
        let expected_anchor = Depth::new(&cam).anchor(raised_screen[1], 2.0, 1);
        assert_eq!(
            raised_anchor.depth, expected_anchor.depth,
            "raised Sim depth anchor must match Depth::anchor in terrain steps at {zoom:?}"
        );
    }

    // Find a drawn interactive furniture object pixel and click it to open its pie menu.
    let mut clicked_entry = None;
    let candidate_ids: Vec<u16> = game
        .lot()
        .world()
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_none())
        .map(|(&id, _)| id)
        .collect();
    'search: for obj_id in candidate_ids {
        let Some(rect) = game.canvas().drawn(obj_id) else {
            continue;
        };
        let cx = (rect[0] + rect[2]) / 2;
        let cy = (rect[1] + rect[3]) / 2;
        for &(px, py) in &[
            (cx, cy),
            (rect[0] + 2, rect[1] + 2),
            (rect[2] - 2, rect[3] - 2),
            (cx, rect[3] - 3),
        ] {
            if game.canvas().id_at(px, py) != Some(obj_id) {
                continue;
            }
            game.hover([px, py]);
            let _ = game.press([px, py]);
            let _ = game.release([px, py]);
            if let Some(menu) = game.menu() {
                let labels: Vec<(String, sims_ui::frame::Rect)> = menu
                    .labels()
                    .into_iter()
                    .map(|(s, r)| (s.to_owned(), r))
                    .collect();
                if !labels.is_empty() {
                    clicked_entry = Some((obj_id, px, py, labels));
                    break 'search;
                }
            }
        }
    }
    let (clicked_obj, px, py, labels) =
        clicked_entry.expect("clicking a drawn open-pack object must open a non-empty pie menu");
    eprintln!(
        "open-pack pie menu opened on object {clicked_obj} at ({px}, {py}) with entries: {:?}",
        labels.iter().map(|(s, _)| s).collect::<Vec<_>>()
    );
    // Render while pie menu is open to verify clean-room pie menu drawing doesn't panic or error.
    game.render();
    let label_rect = labels[0].1;
    let pick_pt = [
        label_rect.x + label_rect.width / 2,
        label_rect.y + label_rect.height / 2,
    ];
    game.hover(pick_pt);
    let _ = game.press(pick_pt);
    let _ = game.release(pick_pt);
    assert!(
        game.menu().is_none(),
        "clicking pie menu entry must close menu and queue interaction"
    );

    // Step 30 simulation ticks in strict mode and assert 0 VM traps.
    let ran = game.run(30);
    assert_eq!(ran, 30, "all 30 ticks must complete cleanly");
    assert!(
        game.stopped().is_none(),
        "open-pack simulation must not stop: {:?}",
        game.stopped()
    );
    assert!(
        game.errors().is_empty(),
        "open-pack simulation must have 0 VM traps: {:?}",
        game.errors()
    );

    // Exercise camera pan.
    assert!(game.apply(Move::Scroll([24, -16])));
    game.render();

    // Exercise Buy Mode in open-pack (catalog rendering, move_objects overlap placement,
    // and Buy/Build undo preservation across native rosebud & set_hour cheats).
    game.set_mode(Mode::Buy);
    game.render();
    let open_obj = game
        .lot()
        .world()
        .objects
        .iter()
        .find(|(_, o)| {
            o.person.is_none()
                && o.multipart.is_none()
                && o.position[2] == 1
                && o.position[0] > 32
                && o.position[1] > 32
                && ![
                    0x2f72_6fff,
                    0x0baa_71c5,
                    0x0000_0437,
                    0x0000_0439,
                    0xf3aa_a62f,
                ]
                .contains(&o.guid)
        })
        .map(|(_, o)| (o.guid, [o.position[0], o.position[1]]))
        .expect("open-pack cottage must have a placed single-tile furniture object");

    assert!(!game.cheat_move_objects());
    let mut cursor_off = game
        .lot_mut()
        .buy_start(open_obj.0, 1, 0)
        .expect("open-pack buy_start with move_objects off")
        .expect("open-pack buy_start cursor");
    assert!(
        !game
            .lot_mut()
            .buy_move(&mut cursor_off, open_obj.1, 1)
            .expect("open-pack buy_move with move_objects off"),
        "move_objects off must reject overlapping placement in open-pack Buy Mode"
    );
    let _ = game.lot_mut().buy_cancel(&cursor_off);

    submit_cheat(&mut game, "move_objects on");
    assert!(game.cheat_move_objects());
    assert!(game.lot().move_objects());
    let mut buy_cursor = game
        .lot_mut()
        .buy_start(open_obj.0, 1, 0)
        .expect("open-pack buy_start")
        .expect("open-pack buy_start cursor");
    assert!(
        game.lot_mut()
            .buy_move(&mut buy_cursor, open_obj.1, 1)
            .expect("open-pack buy_move with move_objects on"),
        "move_objects on must permit overlapping placement in open-pack Buy Mode"
    );
    game.lot_mut()
        .buy_commit(&buy_cursor)
        .expect("open-pack buy_commit");
    assert!(
        game.lot().can_undo(),
        "Buy Mode commit must populate undo stack"
    );

    // Exercise live cheats (`rosebud;!;!`, `set_hour 21`, `move_objects off`).
    let funds_before = game.lot().world().budget.funds;
    submit_cheat(&mut game, "rosebud;!;!");
    assert_eq!(
        game.lot().world().budget.funds,
        funds_before + 3_000,
        "rosebud;!;! must add $3,000 natively"
    );
    assert!(
        game.lot().can_undo(),
        "native add_funds must preserve Buy/Build undo stack"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native add_funds must not round-trip through Lot::restore"
    );

    submit_cheat(&mut game, "set_hour 21");
    assert_eq!(game.hour(), 21, "set_hour 21 must set clock to 21:00");
    assert!(
        game.lot().can_undo(),
        "native set_hour must preserve Buy/Build undo stack"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native set_hour must not round-trip through Lot::restore"
    );

    submit_cheat(&mut game, "move_objects off");
    assert!(!game.cheat_move_objects());
    assert!(!game.lot().move_objects());
    game.set_mode(Mode::Live);
    game.render();

    // Exercise synthesized WAV playback through `Mixer::open_pack`.
    let wavs = game
        .open_pack_sounds()
        .expect("open-pack must carry synthesized WAV bank");
    assert!(!wavs.is_empty(), "open-pack WAV bank must be non-empty");
    let first_sound_name = wavs.keys().next().expect("at least one WAV").clone();
    let mut mixer = Mixer::open_pack(wavs);
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 0,
        name: first_sound_name.clone(),
    }]));
    assert_eq!(
        mixer.active_open_pack_clips(),
        1,
        "triggering open-pack WAV {first_sound_name:?} must start a clip"
    );
    let pcm = mixer.render(1024);
    assert_eq!(pcm.len(), 2048);
    assert!(
        pcm.iter().any(|&s| s != 0),
        "synthesized open-pack WAV clip must render non-silent PCM audio"
    );
}

#[test]
fn open_pack_two_packs_isolate_save_slots_and_custom_archive_names_boot() {
    let tmp = TempTestDir::new("two-packs-slots");
    let src_pack = open_pack_path();
    let pack_a = tmp.path().join("pack-alpha");
    let pack_b = tmp.path().join("pack-beta");
    copy_dir_recursive(&src_pack, &pack_a);
    copy_dir_recursive(&src_pack, &pack_b);

    // Customize pack A's archive_name to collide with `open_indicators.iff` (no `.far`!)
    // and pack B's archive_name to omit `.far` and use lot_number = 2.
    let pack_toml_a = std::fs::read_to_string(pack_a.join("pack.toml")).unwrap();
    std::fs::write(
        pack_a.join("pack.toml"),
        pack_toml_a
            .replace("open-pack.far", "open_indicators.iff")
            .replace("open-pack", "alpha-pack"),
    )
    .unwrap();
    let pack_toml_b = std::fs::read_to_string(pack_b.join("pack.toml")).unwrap();
    std::fs::write(
        pack_b.join("pack.toml"),
        pack_toml_b
            .replace("open-pack.far", "beta_objects_no_ext")
            .replace("open-pack", "beta-pack"),
    )
    .unwrap();
    let house_toml_b = std::fs::read_to_string(pack_b.join("starter-house.toml")).unwrap();
    std::fs::write(
        pack_b.join("starter-house.toml"),
        house_toml_b.replace("lot_number = 1", "lot_number = 2"),
    )
    .unwrap();

    // Unambiguous length-prefixed encoding prevents delimiter collisions like
    // (`/tmp/a`, `b:c`) vs (`/tmp/a:b`, `c`) and preserves raw non-UTF-8 path bytes.
    assert_ne!(
        open_pack_save_slot(Path::new("/tmp/a"), "b:c", 1),
        open_pack_save_slot(Path::new("/tmp/a:b"), "c", 1),
        "save slot encoding must be unambiguous against delimiter collisions"
    );
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let p1 = Path::new(OsStr::from_bytes(b"/tmp/pack-\xfe"));
        let p2 = Path::new(OsStr::from_bytes(b"/tmp/pack-\xff"));
        assert_ne!(
            open_pack_save_slot(p1, "same", 1),
            open_pack_save_slot(p2, "same", 1),
            "save slot encoding must distinguish non-UTF-8 raw OS path bytes"
        );
    }

    let mut game_a = Game::open_open_pack(&pack_a, &[], false, Frames::default())
        .expect("pack A with colliding archive_name must still boot cleanly");
    let mut game_b = Game::open_open_pack(&pack_b, &[], false, Frames::default())
        .expect("pack B with extensionless archive_name and lot 2 must boot cleanly");

    let slot_a = game_a
        .open_pack_save_slot()
        .expect("pack A save slot")
        .to_owned();
    let slot_b = game_b
        .open_pack_save_slot()
        .expect("pack B save slot")
        .to_owned();
    assert_ne!(
        slot_a, slot_b,
        "two distinct open-pack directories must never share the same save slot"
    );
    assert!(
        !slot_a.contains("sims-rs-open-pack-"),
        "save slot must derive from source pack, never temporary staging dir: {slot_a}"
    );

    // Save both packs into the SAME `--saves` REDB file with different funds.
    let saves_db = tmp.path().join("shared-saves.redb");
    let store = sims_sim::Snapshots::open(&saves_db).expect("open shared Snapshots store");
    submit_cheat(&mut game_a, "rosebud");
    submit_cheat(&mut game_b, "rosebud;!;!");
    let funds_a = game_a.lot().world().budget.funds;
    let funds_b = game_b.lot().world().budget.funds;
    assert_ne!(funds_a, funds_b);
    assert!(
        game_a.save_to(&store, &slot_a),
        "pack A save_to must succeed"
    );
    assert!(
        game_b.save_to(&store, &slot_b),
        "pack B save_to must succeed"
    );

    // Re-boot both packs fresh (new temp staging dirs) and verify both restore
    // cleanly from the shared store without overwriting or rejecting each other.
    drop(game_a);
    drop(game_b);
    let mut game_a2 = Game::open_open_pack(&pack_a, &[], false, Frames::default()).unwrap();
    let mut game_b2 = Game::open_open_pack(&pack_b, &[], false, Frames::default()).unwrap();
    assert_eq!(game_a2.open_pack_save_slot(), Some(slot_a.as_str()));
    assert_eq!(game_b2.open_pack_save_slot(), Some(slot_b.as_str()));

    let bytes_a = store.load_lot(&slot_a).unwrap().expect("pack A saved lot");
    let bytes_b = store.load_lot(&slot_b).unwrap().expect("pack B saved lot");
    game_a2
        .restore(sims_vm::boot::save::Saved::decode(&bytes_a).unwrap())
        .expect("pack A restore from shared saves file must succeed");
    game_b2
        .restore(sims_vm::boot::save::Saved::decode(&bytes_b).unwrap())
        .expect("pack B restore from shared saves file must succeed");
    assert_eq!(game_a2.lot().world().budget.funds, funds_a);
    assert_eq!(game_b2.lot().world().budget.funds, funds_b);
}

#[test]
fn open_pack_cli_png_renders_non_blank_screenshot_without_sims_install() {
    let tmp = TempTestDir::new("cli-open-pack-png");
    let exe = env!("CARGO_BIN_EXE_sims-game");
    let pack_dir = open_pack_path();

    // 1. Explicit `--open-pack <dir> --answers 3 --png <out.png> --ticks 5 --size 640x480`.
    let out1 = tmp.path().join("explicit-open-pack.png");
    let status1 = Command::new(exe)
        .env_remove("SIMS_INSTALL")
        .arg("--open-pack")
        .arg(&pack_dir)
        .arg("--answers")
        .arg("3")
        .arg("--png")
        .arg(&out1)
        .arg("--ticks")
        .arg("5")
        .arg("--size")
        .arg("640x480")
        .output()
        .expect("run sims-game --open-pack --png");
    let stderr1 = String::from_utf8_lossy(&status1.stderr);
    assert!(
        status1.status.success(),
        "sims-game --open-pack --png failed:\nstdout: {}\nstderr: {stderr1}",
        String::from_utf8_lossy(&status1.stdout),
    );
    assert!(
        stderr1.contains("ran 5 ticks"),
        "expected headless run to report 5 completed ticks, got stderr:\n{stderr1}"
    );
    let png1 = std::fs::read(&out1).expect("read rendered open-pack PNG");
    let (w1, h1, rgb1) = decode_stored_rgb_png(&png1);
    assert_eq!((w1, h1), (640, 480));
    let mut colors1 = BTreeSet::new();
    for &px in rgb1.as_chunks::<3>().0.iter().step_by(7) {
        colors1.insert(px);
    }
    assert!(
        colors1.len() >= 16,
        "decoded 640x480 open-pack PNG must have rich RGB variation (got {} colors)",
        colors1.len()
    );

    // 2. Automatic standalone fallback (no `--open-pack`, no `<install>`, `SIMS_INSTALL` unset).
    let out2 = tmp.path().join("auto-fallback.png");
    let status2 = Command::new(exe)
        .env_remove("SIMS_INSTALL")
        .current_dir(pack_dir.parent().expect("repo root"))
        .arg("--png")
        .arg(&out2)
        .arg("--ticks")
        .arg("2")
        .arg("--size")
        .arg("320x240")
        .output()
        .expect("run sims-game --png auto fallback");
    let stderr2 = String::from_utf8_lossy(&status2.stderr);
    assert!(
        status2.status.success(),
        "sims-game auto fallback --png failed:\nstdout: {}\nstderr: {stderr2}",
        String::from_utf8_lossy(&status2.stdout),
    );
    assert!(
        stderr2.contains("ran 2 ticks"),
        "expected auto-fallback headless run to report 2 ticks, got stderr:\n{stderr2}"
    );
    let png2 = std::fs::read(&out2).expect("read auto-fallback PNG");
    let (w2, h2, rgb2) = decode_stored_rgb_png(&png2);
    assert_eq!((w2, h2), (320, 240));
    let mut colors2 = BTreeSet::new();
    for &px in rgb2.as_chunks::<3>().0.iter().step_by(5) {
        colors2.insert(px);
    }
    assert!(
        colors2.len() >= 12,
        "decoded 320x240 auto-fallback PNG must be non-blank (got {} colors)",
        colors2.len()
    );

    // 3. Headless choice-dialog fixture: fails with non-zero exit & no PNG without
    //    `--answers`, and completes cleanly with a valid decoded PNG when `--answers 2`
    //    is supplied.
    let dialog_pack = tmp.path().join("dialog-pack");
    copy_dir_recursive(&pack_dir, &dialog_pack);
    let mailbox_antics = dialog_pack.join("objects/open-mailbox.antics");
    let orig_antics = std::fs::read_to_string(&mailbox_antics).expect("read mailbox antics");
    let patched_antics = orig_antics.replace(
        "    main {\n        return true;\n    }",
        "    main {\n        if self.paid_bills == 0 {\n            self.paid_bills = 1;\n            dialog(kind: 2, message: 1, title: 1, button1: 0, button2: 0, button3: 0, mode4_str: 0, flags: 1);\n        }\n        sleep(300);\n        return true;\n    }",
    );
    assert_ne!(patched_antics, orig_antics, "patched mailbox Main");
    std::fs::write(&mailbox_antics, patched_antics).expect("write dialog mailbox antics");

    let out_fail = tmp.path().join("dialog-unanswered.png");
    let status_fail = Command::new(exe)
        .env_remove("SIMS_INSTALL")
        .arg("--open-pack")
        .arg(&dialog_pack)
        .arg("--png")
        .arg(&out_fail)
        .arg("--ticks")
        .arg("5")
        .arg("--size")
        .arg("320x240")
        .output()
        .expect("run dialog pack without --answers");
    assert!(
        !status_fail.status.success(),
        "headless run pausing on an unanswered dialog must exit non-zero"
    );
    assert!(
        !out_fail.exists(),
        "incomplete headless run must not write a misleading PNG"
    );

    let out_answered = tmp.path().join("dialog-answered.png");
    let status_ok = Command::new(exe)
        .env_remove("SIMS_INSTALL")
        .arg("--open-pack")
        .arg(&dialog_pack)
        .arg("--answers")
        .arg("2")
        .arg("--png")
        .arg(&out_answered)
        .arg("--ticks")
        .arg("5")
        .arg("--size")
        .arg("320x240")
        .output()
        .expect("run dialog pack with --answers 2");
    let stderr_ok = String::from_utf8_lossy(&status_ok.stderr);
    assert!(
        status_ok.status.success(),
        "headless run with --answers 2 must succeed:\nstderr: {stderr_ok}"
    );
    assert!(
        stderr_ok.contains("ran 5 ticks"),
        "answered headless run must complete all 5 ticks:\n{stderr_ok}"
    );
    let png_ok = std::fs::read(&out_answered).expect("read answered PNG");
    let (w_ok, h_ok, rgb_ok) = decode_stored_rgb_png(&png_ok);
    assert_eq!((w_ok, h_ok), (320, 240));
    let mut colors_ok = BTreeSet::new();
    for &px in rgb_ok.as_chunks::<3>().0.iter().step_by(5) {
        colors_ok.insert(px);
    }
    assert!(colors_ok.len() >= 12);
}

#[test]
fn autosave_worker_sha256_fast_path_commits_and_loads_cleanly() {
    let tmp = TempTestDir::new("autosave-sha256");
    let db_path = tmp.path().join("autosaves.redb");
    let store = AutosaveStore::open(&db_path).expect("open autosave store");
    let worker = AutosaveWorker::spawn(store.clone());

    let world = sims_sim::World::empty();
    let world_bytes = world.encode();
    worker
        .enqueue_job(SaveJob::manual(
            "UserData",
            1,
            42,
            SavePayload::from_world(world),
        ))
        .expect("enqueue valid world job");
    worker.flush();
    let done = worker.poll_completions();
    assert_eq!(done.len(), 1);
    let meta = done[0]
        .result
        .as_ref()
        .expect("valid world save must commit");
    assert_eq!(meta.sim_tick, 42);

    // Load via `load_save_validated_with_sha256` using the cached SHA-256 validator.
    let loaded = store
        .load_save_validated_with_sha256(
            "UserData",
            1,
            SaveSlot::Manual,
            validate_save_payload_cached_with_sha256,
        )
        .expect("load_save_validated_with_sha256 must succeed");
    assert_eq!(loaded.payload, world_bytes);

    // Corrupt SavedLot payload with valid header must still be rejected by the SHA-256 fast path.
    let mut bad_saved = Vec::new();
    bad_saved.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    bad_saved.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    bad_saved.extend_from_slice(b"corrupt-body");
    worker
        .enqueue_job(SaveJob::manual(
            "UserData",
            2,
            10,
            SavePayload::saved_lot(bad_saved),
        ))
        .expect("enqueue bad saved job");
    worker.flush();
    let done2 = worker.poll_completions();
    assert_eq!(done2.len(), 1);
    assert!(
        matches!(done2[0].result, Err(SaveError::DecodeFailed { .. })),
        "corrupt SavedLot body must fail DecodeFailed, got {:?}",
        done2[0].result
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn complete_collection_native_cheats_move_objects_and_undo_preservation() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!(
            "SIMS_INSTALL is not set; skipping complete_collection_native_cheats_move_objects_and_undo_preservation"
        );
        return;
    };
    let tmp = TempTestDir::new("cc-cheats");
    let saves = tmp.path().join("saves.redb");
    let mut shell = NeighborhoodShell::open(root, saves, 1, true, vec![3], Frames::default())
        .expect("open neighborhood shell");
    shell.enter_lot(5).expect("enter Goth lot 5");

    let game = shell.game_mut().expect("active lot game");
    game.set_mode(Mode::Buy);

    // Find an existing single-tile floor object (e.g. a chair/lamp) and its occupied fine position.
    let existing_obj = game
        .lot()
        .world()
        .objects
        .iter()
        .find(|(_, o)| {
            o.person.is_none()
                && o.multipart.is_none()
                && o.position[2] == 1
                && o.position[0] > 32
                && o.position[1] > 32
                && game.lot().can_pick_up(o.id).unwrap_or(false)
        })
        .map(|(&id, o)| (id, o.guid, o.position))
        .expect("Goth lot must have a pickable single-tile object");

    let occupied_fine = [existing_obj.2[0], existing_obj.2[1]];
    // Buy a new copy of the same object to test overlapping placement with move_objects off vs on.
    let mut cursor = game
        .lot_mut()
        .buy_start(existing_obj.1, 1, 0)
        .expect("buy_start of single-tile object")
        .expect("buy_start must yield cursor");
    let placed_off = game
        .lot_mut()
        .buy_move(&mut cursor, occupied_fine, 1)
        .expect("buy_move with move_objects off");
    assert!(
        !placed_off,
        "overlapping placement on occupied tile must be rejected when move_objects is off"
    );
    let _ = game.lot_mut().buy_cancel(&cursor);

    // Enable `move_objects on` via the cheat console and verify honest active feedback & native VM override!
    submit_cheat(game, "move_objects on");
    assert!(game.cheat_move_objects());
    assert!(game.lot().move_objects());
    let fb = game
        .cheat_console()
        .feedback()
        .map(|f| f.message.as_str())
        .unwrap_or("");
    assert!(
        !fb.contains("pending"),
        "move_objects feedback must not say pending: {fb:?}"
    );

    let mut cursor2 = game
        .lot_mut()
        .buy_start(existing_obj.1, 1, 0)
        .expect("buy_start with move_objects on")
        .expect("buy_start cursor2");
    let placed_on = game
        .lot_mut()
        .buy_move(&mut cursor2, occupied_fine, 1)
        .expect("buy_move with move_objects on");
    assert!(
        placed_on,
        "overlapping placement on occupied tile must succeed with move_objects on"
    );
    game.lot_mut()
        .buy_commit(&cursor2)
        .expect("buy_commit must succeed and push onto Buy undo stack");
    assert!(
        game.lot().can_undo(),
        "Buy Mode commit must populate Buy/Build undo stack"
    );

    // Execute `rosebud;!;!` and `set_hour 20` and verify Buy/Build undo stack is PRESERVED!
    let funds_before = game.lot().world().budget.funds;
    submit_cheat(game, "rosebud;!;!");
    assert_eq!(game.lot().world().budget.funds, funds_before + 3_000);
    assert!(
        game.lot().can_undo(),
        "native rosebud must preserve Buy/Build undo history"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native rosebud must not clear neighborhood_origin via Lot::restore"
    );

    submit_cheat(game, "set_hour 20");
    assert_eq!(game.hour(), 20);
    assert!(
        game.lot().can_undo(),
        "native set_hour must preserve Buy/Build undo history"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native set_hour must not clear neighborhood_origin via Lot::restore"
    );

    submit_cheat(game, "move_objects off");
    assert!(!game.cheat_move_objects());
    assert!(!game.lot().move_objects());

    // Native save_current_lot must succeed directly (without staging cheat fallback).
    shell
        .save_current_lot()
        .expect("save_current_lot must succeed after native live cheats");
}

#[test]
#[allow(clippy::disallowed_methods)]
fn complete_collection_warm_enter_lot_vm_cache_parity_and_speedup() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!(
            "SIMS_INSTALL is not set; skipping complete_collection_warm_enter_lot_vm_cache_parity_and_speedup"
        );
        return;
    };
    let tmp = TempTestDir::new("cc-vm-cache");
    let saves = tmp.path().join("saves.redb");
    let mut shell =
        NeighborhoodShell::open(root.clone(), saves, 1, true, vec![3], Frames::default())
            .expect("open neighborhood shell");

    // 1. Full parity on base house boot: step 30 ticks, render, drain sounds, encode save.
    let mut uncached_base = Game::open_from_store_cached(
        &root,
        shell.store(),
        "UserData",
        5,
        &[3],
        true,
        Frames::default(),
        None,
    )
    .expect("uncached base open_from_store_cached");
    shell.enter_lot(5).expect("cold enter_lot 5 (warms cache)");
    shell.enter_lot(5).expect("warm cached base enter_lot 5");
    {
        let cached_base = shell.game_mut().expect("cached base game");
        uncached_base.resize([800, 600]);
        cached_base.resize([800, 600]);
        assert_eq!(uncached_base.run(30), 30);
        assert_eq!(cached_base.run(30), 30);
        uncached_base.render();
        cached_base.render();
        assert_eq!(
            cached_base.lot().world().hash(),
            uncached_base.lot().world().hash(),
            "base cached vs uncached 30-tick World::hash() must be bit-identical"
        );
        assert_eq!(
            cached_base.canvas().color,
            uncached_base.canvas().color,
            "base cached vs uncached rendered Canvas must be bit-identical"
        );
        let s1: Vec<_> = uncached_base.take_sounds();
        let s2: Vec<_> = cached_base.take_sounds();
        assert_eq!(s2, s1, "base cached vs uncached sound events must match");
        assert_eq!(
            cached_base.save().unwrap().encode(),
            uncached_base.save().unwrap().encode(),
            "base cached vs uncached Saved::encode() bytes must be bit-identical"
        );
    }

    // 2. Persist a saved-lot snapshot and verify full parity on saved-lot restoration too!
    shell
        .save_current_lot()
        .expect("save lot 5 into store for saved-lot cache parity check");
    let mut uncached_saved = Game::open_from_store_cached(
        &root,
        shell.store(),
        "UserData",
        5,
        &[3],
        true,
        Frames::default(),
        None,
    )
    .expect("uncached saved open_from_store_cached");
    shell.enter_lot(5).expect("warm cached saved enter_lot 5");
    {
        let cached_saved = shell.game_mut().expect("cached saved game");
        uncached_saved.resize([800, 600]);
        cached_saved.resize([800, 600]);
        assert_eq!(uncached_saved.run(20), 20);
        assert_eq!(cached_saved.run(20), 20);
        uncached_saved.render();
        cached_saved.render();
        assert_eq!(
            cached_saved.lot().world().hash(),
            uncached_saved.lot().world().hash(),
            "saved-lot cached vs uncached World::hash() must be bit-identical"
        );
        assert_eq!(
            cached_saved.canvas().color,
            uncached_saved.canvas().color,
            "saved-lot cached vs uncached Canvas must be bit-identical"
        );
        let s1: Vec<_> = uncached_saved.take_sounds();
        let s2: Vec<_> = cached_saved.take_sounds();
        assert_eq!(
            s2, s1,
            "saved-lot cached vs uncached sound events must match"
        );
        assert_eq!(
            cached_saved.save().unwrap().encode(),
            uncached_saved.save().unwrap().encode(),
            "saved-lot cached vs uncached Saved::encode() bytes must be bit-identical"
        );
    }

    // 3. 5-run median timing comparison (uncached vs warm cached enter_lot).
    let mut uncached_ms = Vec::with_capacity(5);
    for _ in 0..5 {
        let t0 = Instant::now();
        let _g = Game::open_from_store_cached(
            &root,
            shell.store(),
            "UserData",
            5,
            &[3],
            true,
            Frames::default(),
            None,
        )
        .expect("uncached open_from_store_cached");
        uncached_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    uncached_ms.sort_by(f64::total_cmp);
    let uncached_median = uncached_ms[2];

    let mut cached_ms = Vec::with_capacity(5);
    for _ in 0..5 {
        let t0 = Instant::now();
        shell.enter_lot(5).expect("warm enter_lot 5");
        cached_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    cached_ms.sort_by(f64::total_cmp);
    let cached_median = cached_ms[2];

    eprintln!(
        "BENCH warm enter_lot(House05) 5-run median: uncached={uncached_median:.2} ms, cached(VmInstallCache+InstallAssets)={cached_median:.2} ms (speedup={:.2}x)",
        uncached_median / cached_median.max(0.001)
    );
}

#[test]
fn complete_collection_native_caf_surnames_without_sidecar_and_legacy_fallback() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!(
            "SIMS_INSTALL is not set; skipping complete_collection_native_caf_surnames_without_sidecar_and_legacy_fallback"
        );
        return;
    };
    let tmp = TempTestDir::new("cc-caf-surnames");
    let saves = tmp.path().join("saves.redb");
    let sidecar = saves.with_extension("caf-names.tsv");

    // 1. Drive a CAF family commit straight through `NeighborhoodShell` UI handlers
    //    (exercising `NeighborhoodShell::handle_caf_outcome`) and verify native REDB v4
    //    persistence with zero `.caf-names.tsv` sidecar writes.
    let mut shell = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open neighborhood shell");
    shell.resize([800, 600]);
    shell.open_family_bin();
    shell.open_create_family().expect("open CAF");
    {
        let caf = shell.caf_session_mut().expect("CAF session");
        let _ = caf.controller_mut().begin_add_sim();
        {
            let sim = caf.controller_mut().editing_sim_mut().expect("editing sim");
            sim.first_name = "Nova".into();
            sim.bio = "Clean-room REDB v4 surname test".into();
            sim.appearance = sims_ui::caf::SimAppearance {
                age: sims_ui::caf::Age::Adult,
                gender: sims_ui::caf::Gender::Female,
                skin_tone: sims_ui::caf::SkinTone::Light,
                ..sims_ui::caf::SimAppearance::default()
            };
            sim.personality = sims_ui::caf::Personality::from_raw([5, 5, 5, 5, 5]);
        }
    }
    // Click SimDone in EditSim screen through `NeighborhoodShell::press`/`release`.
    let caf_layout = sims_ui::caf::CafLayout::compute(800, 600);
    let sim_done_pt = [
        caf_layout.sim_done.x + caf_layout.sim_done.width / 2,
        caf_layout.sim_done.y + caf_layout.sim_done.height / 2,
    ];
    let _ = shell.press(sim_done_pt);
    let _ = shell.release(sim_done_pt);
    // Click family last-name field and type "Starlight" through `NeighborhoodShell`,
    // then click FamilyDone -> confirm.
    let fam_name_pt = [
        caf_layout.family_name_field.x + caf_layout.family_name_field.width / 2,
        caf_layout.family_name_field.y + caf_layout.family_name_field.height / 2,
    ];
    let _ = shell.press(fam_name_pt);
    let _ = shell.release(fam_name_pt);
    for ch in "Starlight".chars() {
        let _ = shell.key_char(ch);
    }
    let fam_done_pt = [
        caf_layout.family_done.x + caf_layout.family_done.width / 2,
        caf_layout.family_done.y + caf_layout.family_done.height / 2,
    ];
    let _ = shell.press(fam_done_pt);
    let _ = shell.release(fam_done_pt);
    let _ = shell.key_enter(); // accept commit confirmation if shown
    assert!(
        !sidecar.exists(),
        "NeighborhoodShell CAF commit must NOT write legacy .caf-names.tsv sidecar"
    );

    let created_family_id = shell
        .family_bin_entries()
        .iter()
        .find(|e| e.name == "Starlight")
        .map(|e| e.id as u16)
        .expect("Family Bin must contain Starlight committed through NeighborhoodShell");

    // Reopen shell from disk and verify surname survives natively in StoredNeighborhood.family_surnames.
    drop(shell);
    let reopened = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("reopen neighborhood shell");
    let stored = reopened
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.family_surname(created_family_id),
        Some("Starlight"),
        "StoredNeighborhood.family_surnames must persist CAF surname natively"
    );
    assert!(
        reopened
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(created_family_id) && e.name == "Starlight"),
        "Family Bin must display native REDB surname after reopen"
    );

    // 2. Verify one-time read-only legacy `.caf-names.tsv` migration AND that deleting
    //    the migrated family never resurrects its stale sidecar name when the ID is reused.
    let mut stripped = stored.clone();
    stripped.remove_family_surname(created_family_id);
    reopened
        .store()
        .save_neighborhood("UserData", &stripped)
        .expect("save stripped neighborhood");
    let inst_hex = format!("{:032x}", stripped.instance_id);
    let sidecar_body = format!("UserData\t{inst_hex}\t{created_family_id}\tLegacySidecarName\n");
    std::fs::write(&sidecar, &sidecar_body).expect("write legacy sidecar fixture");
    drop(reopened);

    let mut legacy_shell = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("reopen with legacy sidecar");
    assert!(
        legacy_shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(created_family_id) && e.name == "LegacySidecarName"),
        "one-time legacy .caf-names.tsv migration must populate surnames for pre-v4 saves"
    );
    assert_eq!(
        std::fs::read_to_string(&sidecar).unwrap(),
        sidecar_body,
        "legacy .caf-names.tsv file must remain strictly read-only on disk"
    );
    let migrated_stored = legacy_shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap();
    assert_eq!(
        migrated_stored.family_surname(created_family_id),
        Some("LegacySidecarName"),
        "migrated surname must now be persisted natively in REDB"
    );

    // Delete the migrated family and verify that even if a later family reuses its
    // family_id with no native surname, the stale read-only sidecar row is NOT reapplied!
    legacy_shell
        .execute_delete_family(created_family_id)
        .expect("delete migrated family");
    let mut after_del = legacy_shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap();
    assert!(!after_del.families.contains_key(&created_family_id));
    assert!(after_del.family_surname(created_family_id).is_none());
    // Simulate a future family reusing `created_family_id` with no native surname.
    let reused_family = stripped.families.get(&created_family_id).unwrap().clone();
    after_del.families.insert(created_family_id, reused_family);
    legacy_shell
        .store()
        .save_neighborhood("UserData", &after_del)
        .expect("save reused family_id fixture");
    drop(legacy_shell);

    let post_delete_shell =
        NeighborhoodShell::open(root, saves, 1, true, vec![3], Frames::default())
            .expect("reopen after delete + family_id reuse");
    assert!(
        !post_delete_shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(created_family_id) && e.name == "LegacySidecarName"),
        "deleted legacy family name must NOT be reapplied from read-only sidecar after migration"
    );
}
