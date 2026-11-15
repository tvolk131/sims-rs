//! The ambient-score player on hand-built scores and samples.

use sims_audio::ambience;
use sims_audio::fsc::Score;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE};
use sims_audio::{Pcm, SoundEvent};
use std::collections::BTreeMap;

/// A module with no routine to speak of and the events the ambience
/// needs.
fn module() -> Module {
    let mut hit = b"HIT!".to_vec();
    hit.extend(1u32.to_le_bytes());
    hit.extend(8u32.to_le_bytes());
    hit.extend(b"TRAX");
    hit.push(12);
    let hot = "[EventMapping]\n\
               amb0=52,0\namb1=52,1\namb9=52,9\n\
               music0=36,0\nmusic3=36,3\nmusic7=36,7\n\
               killall=20\n";
    Module::parse("test", &hit, b"", hot.as_bytes()).unwrap()
}

/// One cell: `vol rand? LR FB rand? fin fout dly rand? loop loop? quant
/// prob pitchL pitchH fast? groupID stereo?`, then its file and any
/// exclusions.
#[derive(Clone)]
struct Cell {
    fields: [i64; 18],
    file: &'static str,
    exclusions: &'static str,
}

fn cell(prob: i64, file: &'static str) -> Cell {
    Cell {
        fields: [
            1024, 0, 512, 512, 0, 0, 0, 0, 0, 8, 0, 0, prob, 0, 0, 0, 0, 0,
        ],
        file,
        exclusions: "",
    }
}

/// A score's text: `vol pri min max rows cols tempo BPB SelX SelY QuanX
/// QuanY DifX DifY`, then the cells row by row (`None`: empty).
fn score(header: [i64; 14], cells: &[Option<Cell>]) -> Score {
    let mut text = String::from("version 7\n#\tvol\tpri\n");
    for value in header {
        text.push_str(&format!("\t{value}"));
    }
    text.push_str("\n#\tvol\n");
    for cell in cells {
        match cell {
            Some(cell) => {
                for value in cell.fields {
                    text.push_str(&format!("\t{value}"));
                }
                text.push_str(&format!("\t{}\t{}\n", cell.file, cell.exclusions));
            }
            None => {
                text.push_str(&"\t0".repeat(18));
                text.push_str("\tNONE\n");
            }
        }
    }
    Score::parse(text.as_bytes()).unwrap()
}

/// `rows` by `cols`, tempo 60 (a beat a second), 2 beats a bar, `min`
/// and `max`, a window `half` cells either side.
fn header(rows: i64, cols: i64, min: i64, max: i64, half: i64) -> [i64; 14] {
    [1024, 0, min, max, rows, cols, 60, 2, half, half, 1, 1, 0, 0]
}

/// Samples of 1,000 frames at the output rate: `a.xa` holds 1,000s,
/// `b.xa` 100s.
fn samples() -> BTreeMap<String, Pcm> {
    let mut map = BTreeMap::new();
    for (name, value) in [("amb\\a.xa", 1000), ("amb\\b.xa", 100)] {
        map.insert(
            name.to_owned(),
            Pcm {
                rate: RATE,
                channels: 1,
                samples: vec![value; 1000],
            },
        );
    }
    map
}

fn player(scores: &[(u32, Score)]) -> Player<BTreeMap<String, Pcm>> {
    let mut player = Player::new(vec![module()], samples(), 7);
    for (mode, score) in scores {
        assert!(player.add_score(*mode, score, "amb"));
    }
    player
}

fn event(player: &mut Player<BTreeMap<String, Pcm>>, name: &str) {
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: name.into(),
        },
        &NoSources,
    );
}

/// The frames (stereo pairs) from `from` on where either channel is not
/// 0, as runs: first frame and length.
fn runs(stereo: &[i16], from: usize) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (frame, pair) in stereo.chunks(2).enumerate() {
        if pair != [0, 0] {
            match runs.last_mut() {
                Some((start, length)) if *start + *length == frame + from => *length += 1,
                _ => runs.push((frame + from, 1)),
            }
        }
    }
    runs
}

/// The first block start at or after a time in milliseconds.
fn block_at(ms: u64) -> usize {
    let frame = (ms * u64::from(RATE)).div_ceil(1000) as usize;
    frame.div_ceil(BLOCK) * BLOCK
}

#[test]
fn a_cell_plays_on_the_beat_after_it_is_chosen() {
    let center = [
        None,
        None,
        None,
        None,
        Some(cell(20, "a.xa")),
        None,
        None,
        None,
        None,
    ];
    let mut player = player(&[(0, score(header(3, 3, 1, 1, 1), &center))]);
    event(&mut player, "amb0");
    assert!(player.ambience().is_some_and(|a| a.is_started()));
    let out = player.render_stereo(8 * RATE as usize, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // Beat 2 at once; beat 1 at 1 s chooses the cell and loads it; it
    // plays on the next beat 1, at 3 s, at full volume and centred; at
    // 4 s it is idle, at 5 s chosen again, at 7 s it plays again.
    assert_eq!(
        runs(&out, 0),
        vec![(block_at(3000), 1000), (block_at(7000), 1000)]
    );
    let first = block_at(3000) * 2;
    assert_eq!(&out[first..first + 4], &[1000, 1000, 1000, 1000]);
}

#[test]
fn a_cell_outside_the_window_waits_for_the_cursor() {
    let mut cells = vec![None; 9];
    cells[8] = Some(cell(20, "a.xa"));
    let mut player = player(&[(0, score(header(3, 3, 1, 1, 0), &cells))]);
    event(&mut player, "amb0");
    // Starting puts the cursor at (0, 0): the window is that cell alone.
    assert_eq!(player.ambience().unwrap().cursor(), (0, 0));
    let out = player.render_stereo(5 * RATE as usize, &NoSources);
    assert!(runs(&out, 0).is_empty());
    // A view of 100 takes the last row; 11 pm, column 12 of 13, the last
    // column.
    player.set_ambience_view(100, 23);
    assert_eq!(player.ambience().unwrap().cursor(), (2, 2));
    let from = 5 * RATE as usize;
    let out = player.render_stereo(5 * RATE as usize, &NoSources);
    // The beat at 5 s falls in the next block: chosen then, played at 7 s.
    assert_eq!(runs(&out, from), vec![(block_at(7000), 1000)]);
}

#[test]
fn the_view_and_the_hour_move_the_cursor() {
    let cells = vec![None; 26 * 14];
    let mut player = player(&[(0, score(header(26, 14, 1, 1, 0), &cells))]);
    event(&mut player, "amb0");
    // Below 30 the view counts as 0. Noon is column 6: 6000 / 13 = 461 per
    // mille of 14 columns, 6.
    player.set_ambience_view(29, 12);
    assert_eq!(player.ambience().unwrap().cursor(), (0, 6));
    // 50 is 500 per mille of 26 rows, 13; 5 am is column 1, 76 per mille.
    player.set_ambience_view(50, 5);
    assert_eq!(player.ambience().unwrap().cursor(), (13, 1));
    // An hour past 23 leaves the column.
    player.set_ambience_view(50, 24);
    assert_eq!(player.ambience().unwrap().cursor(), (13, 1));
}

#[test]
fn pitch_is_semitones_of_the_samples_own_rate() {
    for (semitones, length) in [(12, 500), (-12, 2000)] {
        let mut up = cell(20, "a.xa");
        up.fields[13] = semitones;
        up.fields[14] = semitones;
        let mut player = player(&[(0, score(header(1, 1, 1, 1, 0), &[Some(up)]))]);
        event(&mut player, "amb0");
        let out = player.render_stereo(4 * RATE as usize, &NoSources);
        assert_eq!(runs(&out, 0), vec![(block_at(3000), length)], "{semitones}");
    }
}

#[test]
fn lr_pans_the_cell() {
    let mut left = cell(20, "a.xa");
    left.fields[2] = 0;
    let mut player = player(&[(0, score(header(1, 1, 1, 1, 0), &[Some(left)]))]);
    event(&mut player, "amb0");
    let out = player.render_stereo(4 * RATE as usize, &NoSources);
    let first = block_at(3000) * 2;
    assert_eq!(&out[first..first + 2], &[1000, 0]);
}

#[test]
fn the_effects_volume_is_the_ambience_volume() {
    let mut player = player(&[(0, score(header(1, 1, 1, 1, 0), &[Some(cell(20, "a.xa"))]))]);
    player.set_group_volume(1, 512);
    event(&mut player, "amb0");
    let out = player.render_stereo(4 * RATE as usize, &NoSources);
    let first = block_at(3000) * 2;
    // 512 is -20 dB in the sound layer: a gain of a tenth.
    let expected = ((1000i64 * i64::from(sims_audio::levels::gain(-2000))) >> 16) as i16;
    assert_eq!(&out[first..first + 2], &[expected, expected]);
    assert_eq!(expected, 99);
}

#[test]
fn cells_of_a_group_and_cells_excluded_never_play_together() {
    let both = |a: Cell, b: Cell| [Some(a), Some(b)];
    let mut grouped = (cell(20, "a.xa"), cell(20, "a.xa"));
    grouped.0.fields[16] = 1;
    grouped.1.fields[16] = 1;
    let mut excluded = (cell(20, "a.xa"), cell(20, "a.xa"));
    excluded.0.exclusions = "(1,0)";
    excluded.1.exclusions = "(0,0)";
    let free = (cell(20, "a.xa"), cell(20, "a.xa"));
    for (name, (a, b), together) in [
        ("grouped", grouped, false),
        ("excluded", excluded, false),
        ("free", free, true),
    ] {
        let mut player = player(&[(0, score(header(1, 2, 2, 2, 1), &both(a, b)))]);
        event(&mut player, "amb0");
        let out = player.render_stereo(20 * RATE as usize, &NoSources);
        let loudest = out.iter().copied().max().unwrap();
        assert_eq!(loudest == 2000, together, "{name}: {loudest}");
        assert!(loudest >= 1000, "{name}");
    }
}

#[test]
fn a_new_mode_stops_the_old_score_and_starts_its_own() {
    let one = score(header(1, 1, 1, 1, 0), &[Some(cell(20, "a.xa"))]);
    let two = score(header(1, 1, 1, 1, 0), &[Some(cell(20, "b.xa"))]);
    let mut player = player(&[(0, one), (1, two)]);
    event(&mut player, "amb0");
    let out = player.render_stereo(block_at(3000) + 100, &NoSources);
    assert_eq!(out[out.len() - 2..], [1000, 1000]);
    event(&mut player, "amb1");
    // The backyard's cell stopped at once; the beach starts its beat.
    let from = out.len() / 2;
    let out = player.render_stereo(4 * RATE as usize, &NoSources);
    let runs = runs(&out, from);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(out[(runs[0].0 - from) * 2], 100);
    assert!(player.ambience().is_some_and(|a| a.mode() == 1));
    // The same mode again changes nothing; a mode with no score stops the
    // current one and leaves none.
    event(&mut player, "amb1");
    assert!(player.ambience().is_some_and(|a| a.is_started()));
    event(&mut player, "amb9");
    assert!(player.ambience().is_none());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn killing_all_stops_the_ambience_and_the_music_mode_starts_it() {
    let one = score(header(1, 1, 1, 1, 0), &[Some(cell(20, "a.xa"))]);
    let mut player = player(&[(0, one)]);
    // Before the first neighbourhood mode, modes are not recorded: the
    // first mode stays 4, which stops the (absent) ambience.
    event(&mut player, "music0");
    assert!(player.ambience().is_none());
    // The neighbourhood makes the backyard score current and starts it.
    event(&mut player, "music3");
    assert!(player.ambience().is_some_and(|a| a.is_started()));
    player.render_stereo(block_at(3000) + 100, &NoSources);
    assert!(!player.is_idle());
    event(&mut player, "killall");
    assert!(player.ambience().is_some_and(|a| !a.is_started()));
    assert!(player.is_idle());
    // Live mode starts the current ambience again; another mode stops it.
    event(&mut player, "music0");
    assert!(player.ambience().is_some_and(|a| a.is_started()));
    event(&mut player, "music7");
    assert!(player.ambience().is_some_and(|a| !a.is_started()));
    // The music half asks for music tracks this module does not have:
    // the neighbourhood's music and river, then live's slow fade. Mode 7
    // starts none.
    assert_eq!(
        player.faults,
        ["no track 517", "no track 2957", "no track 519"]
    );
}

#[test]
fn a_seed_replays_the_same_ambience() {
    let mut random = cell(9, "a.xa");
    random.fields[1] = 1;
    random.fields[4] = 1;
    random.fields[13] = -3;
    random.fields[14] = 2;
    let cells: Vec<_> = (0..12)
        .map(|i| (i % 3 != 0).then(|| random.clone()))
        .collect();
    let render = |seed| {
        let mut player = Player::new(vec![module()], samples(), seed);
        player.add_score(0, &score(header(3, 4, 2, 3, 1), &cells), "amb");
        event(&mut player, "amb0");
        player.render_stereo(30 * RATE as usize, &NoSources)
    };
    let one = render(1);
    assert_eq!(one, render(1));
    assert_ne!(one, render(2));
    assert!(one.iter().any(|&s| s != 0));
}

#[test]
fn odd_scores_neither_panic_nor_hang() {
    const MIN: i64 = i32::MIN as i64;
    const MAX: i64 = i32::MAX as i64;
    let mut odd = cell(99, "a.xa");
    odd.fields[0] = i64::from(i32::MAX);
    odd.fields[2] = i64::from(i32::MIN);
    odd.fields[13] = -40;
    odd.fields[14] = 40;
    odd.fields[10] = 1;
    odd.fields[9] = -1;
    odd.fields[15] = 1;
    odd.fields[16] = 31;
    odd.exclusions = "(-1,-1)(7,700)";
    let headers = [
        [1024, 0, 1, 1, 2, 2, 0, 2, 1, 1, 1, 1, 0, 0],
        [1024, 1, 5, 9, 2, 2, 60, 0, -1, -3, 1, 1, 7, -7],
        [
            i64::from(i32::MIN),
            1,
            -1,
            99,
            2,
            2,
            -60,
            1,
            9,
            9,
            0,
            0,
            0,
            0,
        ],
        [1024, 1, 2, 2, 2, 2, 100_000, 3, 0, 0, 1, 1, 9, 9],
        [
            1024,
            1,
            2,
            2,
            2,
            2,
            60,
            i64::from(i32::MAX),
            MIN,
            MIN,
            1,
            1,
            MIN,
            MIN,
        ],
        [1024, 1, 2, 2, 2, 2, 60, MIN, MAX, MAX, 1, 1, MAX, MAX],
    ];
    for header in headers {
        let cells = vec![Some(odd.clone()), None, Some(cell(20, "missing.xa")), None];
        let mut player = player(&[(0, score(header, &cells))]);
        event(&mut player, "amb0");
        player.set_ambience_view(i32::MAX, 3);
        player.render_stereo(3 * RATE as usize, &NoSources);
        player.set_ambience_view(i32::MIN, 0);
        player.render_stereo(3 * RATE as usize, &NoSources);
    }
    let empty = score([1024, 0, 1, 1, 0, 0, 60, 2, 1, 1, 1, 1, 0, 0], &[]);
    let mut player = player(&[(0, empty)]);
    event(&mut player, "amb0");
    player.render_stereo(3 * RATE as usize, &NoSources);
}

#[test]
fn a_score_larger_than_the_grid_is_refused() {
    let mut player = Player::new(vec![module()], samples(), 1);
    let cells = vec![None; 65];
    let large = score([1024, 0, 1, 1, 65, 1, 60, 2, 1, 1, 1, 1, 0, 0], &cells);
    assert!(!player.add_score(0, &large, "amb"));
    assert_eq!(player.faults.len(), 1);
}

#[test]
fn a_score_added_later_takes_the_effects_volume() {
    let mut player = Player::new(vec![module()], samples(), 7);
    player.set_group_volume(1, 512);
    let one = score(header(1, 1, 1, 1, 0), &[Some(cell(20, "a.xa"))]);
    assert!(player.add_score(0, &one, "amb"));
    event(&mut player, "amb0");
    let out = player.render_stereo(4 * RATE as usize, &NoSources);
    let first = block_at(3000) * 2;
    assert_eq!(&out[first..first + 2], &[99, 99]);
}

#[test]
fn a_score_replacing_the_current_one_plays_on() {
    let one = score(header(1, 1, 1, 1, 0), &[Some(cell(20, "a.xa"))]);
    let mut player = player(&[(0, one.clone())]);
    event(&mut player, "amb0");
    player.render_stereo(RATE as usize, &NoSources);
    assert!(player.add_score(0, &one, "amb"));
    assert!(player.ambience().is_some_and(|a| a.is_started()));
    let out = player.render_stereo(8 * RATE as usize, &NoSources);
    assert!(!runs(&out, 0).is_empty());
}

#[test]
fn a_looping_cell_whose_sound_is_dropped_frees_its_slot() {
    let mut samples = samples();
    samples.insert(
        "amb\\empty.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: Vec::new(),
        },
    );
    let mut looping = cell(20, "empty.xa");
    looping.fields[10] = 1;
    let one = score(header(1, 1, 1, 1, 0), &[Some(looping)]);
    let mut player = Player::new(vec![module()], samples, 7);
    assert!(player.add_score(0, &one, "amb"));
    event(&mut player, "amb0");
    let mut active = Vec::new();
    for _ in 0..16 {
        player.render_stereo(RATE as usize / 2, &NoSources);
        active.push(player.ambience().unwrap().active());
    }
    // Chosen, loaded, played, then let go when its voice is gone.
    let first = active.iter().position(|&a| a == 1).unwrap();
    assert!(active[first..].contains(&0), "{active:?}");
}

#[test]
fn an_update_far_in_the_future_does_not_overflow() {
    use sims_audio::ambience::{Fresh, Host, Key, Voice};
    struct Silent;
    impl Host for Silent {
        fn rand(&mut self) -> i32 {
            0
        }
        fn load(&mut self, _: &str) -> Option<u32> {
            None
        }
        fn play(&mut self, _: Voice) {}
        fn set_volume(&mut self, _: Key, _: i32) {}
        fn stop(&mut self, _: Key) {}
        fn playing(&self, _: Key) -> bool {
            false
        }
    }
    let one = score(header(1, 1, 1, 1, 0), &[Some(cell(20, "a.xa"))]);
    let mut fresh = Fresh::new(0, &one, "amb", RATE).unwrap();
    fresh.start(0);
    fresh.update(u64::MAX, &mut Silent);
    fresh.update(u64::MAX, &mut Silent);
    assert!(fresh.is_started());
}

/// The view tiles [`ambience::view`] visits, in order, for a screen of
/// `columns` tiles across and `rows` half tiles down at the near zoom.
fn walk(corner: [i32; 2], columns: i32, rows: i32) -> Vec<[i8; 2]> {
    let mut seen = Vec::new();
    ambience::view([columns * 128, rows * 32], 3, corner, |tile| {
        seen.push(tile);
        false
    });
    seen
}

#[test]
fn the_view_walks_the_screen_in_half_tile_rows() {
    // Along a row the column goes up and the row down; each row starts
    // a step down, by the row first and then the column.
    assert_eq!(
        walk([10, 10], 2, 4),
        [
            [10, 10],
            [11, 9],
            [10, 11],
            [11, 10],
            [11, 11],
            [12, 10],
            [11, 12],
            [12, 11],
        ]
    );
    // Tiles beyond 1 to 62 are not asked about (nor counted).
    assert_eq!(walk([0, 2], 3, 1), [[1, 1]]);
    assert_eq!(walk([62, 5], 3, 1), [[62, 5]]);
    // Coordinates wrap as signed bytes.
    assert!(walk([127, 10], 2, 1).is_empty());
    assert_eq!(walk([56 - 256, 10], 1, 1), [[56, 10]]);
}

#[test]
fn the_view_is_the_outdoor_share_in_percent() {
    // Every other tile outside.
    let view = |screen, zoom| ambience::view(screen, zoom, [20, 20], |[x, _]| x % 2 == 0);
    assert_eq!(view([800, 600], 3), 50);
    // All outside, none outside.
    assert_eq!(ambience::view([800, 600], 1, [20, 20], |_| true), 100);
    assert_eq!(ambience::view([800, 600], 2, [20, 20], |_| false), 0);
    // One of three: 33.33... cut to 33.
    let mut n = 0;
    let third = ambience::view([3 * 128, 32], 3, [20, 20], |_| {
        n += 1;
        n == 1
    });
    assert_eq!(third, 33);
    // Nothing on the lot, or no screen: 0, as the original's NaN gives.
    assert_eq!(ambience::view([800, 600], 3, [-100, -100], |_| true), 0);
    assert_eq!(ambience::view([0, 0], 3, [20, 20], |_| true), 0);
    assert_eq!(ambience::view([800, 600], 40, [20, 20], |_| true), 0);
    assert_eq!(
        ambience::view([i32::MIN, i32::MAX], 1, [20, 20], |_| true),
        0
    );
}

#[test]
fn a_pause_holds_the_score_in_live_music_mode_only() {
    let cells = vec![Some(cell(20, "a.xa")); 9];
    let mut player = player(&[(0, score(header(3, 3, 1, 1, 1), &cells))]);
    event(&mut player, "amb0");
    // The first music mode is 4: a pause leaves the score alone, though
    // an unpause would unpause it.
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(player.is_paused());
    assert!(!player.ambience().unwrap().is_paused());
    player.handle(&SoundEvent::Unpause, &NoSources);
    // Live music (mode 0, once mode 3 has been set): the score pauses,
    // and no beat falls due while it is.
    event(&mut player, "music3");
    event(&mut player, "music0");
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(
        player
            .ambience()
            .is_some_and(|a| a.is_paused() && a.is_started())
    );
    let out = player.render_stereo(8 * RATE as usize, &NoSources);
    assert!(runs(&out, 0).is_empty(), "{:?}", runs(&out, 0));
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert!(!player.ambience().unwrap().is_paused());
    let out = player.render_stereo(8 * RATE as usize, &NoSources);
    assert!(!runs(&out, 0).is_empty());
    // Only the music tracks this module does not have.
    assert_eq!(
        player.faults,
        ["no track 517", "no track 2957", "no track 519"]
    );
}
