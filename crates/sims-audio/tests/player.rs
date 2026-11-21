//! The HIT player on hand-built modules and samples.

use sims_audio::module::Module;
use sims_audio::placement::Projection;
use sims_audio::player::{BLOCK, MIDDLE, NoSources, Player, RATE, Sources};
use sims_audio::sources::Snapshot;
use sims_audio::stations::{FIRST_ID, StationLists};
use sims_audio::{Pcm, SoundEvent};
use std::collections::BTreeMap;

/// Assembles a program from routines placed one after another; returns
/// the bytes and each routine's address.
fn assemble(routines: &[&[u8]]) -> (Vec<u8>, Vec<u32>) {
    let mut bytes = b"HIT!".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(8u32.to_le_bytes());
    bytes.extend(b"TRAX");
    let mut addresses = Vec::new();
    for routine in routines {
        addresses.push(bytes.len() as u32);
        bytes.extend(*routine);
    }
    (bytes, addresses)
}

fn long(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

const NOTE_ON_V1: [u8; 2] = [2, 5];
const WAIT_SAMP: u8 = 11;
const END: u8 = 12;

/// A module whose routine `i` has track data ID `0x100 + i` and is run by
/// track `10 + i` (with `kArgsIdVolPan` arguments) and event `e{i}`.
/// Patches 1 and 2 are `one.xa` and `two.xa`; hit list 5 holds both.
fn module(routines: &[&[u8]], patch: u32) -> Module {
    module_with(routines, patch, 10, "")
}

/// [`module`], with tracks numbered from `first` (events still `e{i}`)
/// and more lines in its `[EventMapping]` section. Hit list 6 holds
/// patch 2 alone.
fn module_with(routines: &[&[u8]], patch: u32, first: u32, events: &str) -> Module {
    let (hit, addresses) = assemble(routines);
    let mut hot = String::from("[EventMappingEquate]\nkSndobPlay=1\n[TrackData]\n");
    for (i, address) in addresses.iter().enumerate() {
        hot.push_str(&format!("{:#x}={address:#x}\n", 0x100 + i));
    }
    hot.push_str("[Patch]\n1=0,one,\"one.xa\"\n2=0,two,\"two.xa\"\n");
    hot.push_str("[GlobalHitList]\n5=1,2\n6=2\n[Track]\n");
    for i in 0..routines.len() {
        hot.push_str(&format!(
            "{}=0,t{i},1024,2,0,0,0,{:#x},{patch}\n",
            first as usize + i,
            0x100 + i
        ));
    }
    hot.push_str("[EventMapping]\n");
    for i in 0..routines.len() {
        hot.push_str(&format!("e{i}=kSndobPlay,{}\n", first as usize + i));
    }
    hot.push_str("other=7,10\n");
    hot.push_str(events);
    Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap()
}

fn samples() -> BTreeMap<String, Pcm> {
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: (1..=100).collect(),
        },
    );
    map.insert(
        "two.xa".to_owned(),
        Pcm {
            rate: RATE / 2,
            channels: 2,
            samples: vec![1000, 3000, 3000, 5000, -2000, -2000],
        },
    );
    map
}

fn play(player: &mut Player<BTreeMap<String, Pcm>>, name: &str) {
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: name.into(),
        },
        &NoSources,
    );
}

#[test]
fn a_note_plays_its_patch_and_the_thread_waits_for_it() {
    let generic: &[u8] = &[NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic], 1)], samples(), 1);
    play(&mut player, "E0");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // The sample, then silence for the rest of the block in which the
    // thread saw the note end.
    assert_eq!(&out[..100], &(1..=100).collect::<Vec<i16>>());
    assert_eq!(out.len(), 2 * BLOCK);
    assert!(out[100..].iter().all(|&s| s == 0));
    assert!(player.is_idle());
}

#[test]
fn samples_play_at_the_pitch_rate_whatever_their_own() {
    // two.xa is 11,025 Hz stereo: (1000, 3000), (3000, 5000),
    // (-2000, -2000). At pitch 3600 it plays at 22,050 Hz all the same.
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic], 2)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render_stereo(4, &NoSources);
    assert_eq!(out, [1000, 3000, 3000, 5000, -2000, -2000, 0, 0]);
    // Pitch 2400 is an octave down: 11,025 Hz. Every other output frame
    // is halfway between two; the last frame holds. Mono is the mean.
    let mut routine = vec![LOADL, 21];
    routine.extend(long(2400));
    routine.extend(generic);
    let mut player = Player::new(vec![module(&[&routine], 2)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render(8, &NoSources);
    assert_eq!(out, [2000, 3000, 4000, 1000, -2000, -2000, 0, 0]);
}

#[test]
fn notes_sum_and_clamp() {
    let mut map = samples();
    map.insert(
        "one.xa".into(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![30000; 4],
        },
    );
    // Two tracks: a second play of one track that runs starts nothing.
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic, generic], 1)], map, 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    assert_eq!(
        player.render(5, &NoSources),
        [32767, 32767, 32767, 32767, 0]
    );
}

#[test]
fn calls_return_at_end_and_branches_follow_compares() {
    // Routine 0: v2 = 3; call routine 1; if v2 == 4 play patch 1; end.
    // Routine 1: v3 = 1; v2 += v3; end (returns).
    let main: Vec<u8> = [
        &[4, 6, 3][..], // loadb v2, 3
        &[7],
        &long(0x101), // call 0x101
        &[4, 7, 4],   // loadb v3, 4
        &[20, 6, 7],  // cmp v2, v3
        &[63],
        &long(0x102),       // ifne 0x102 (end)
        &[2, 5, WAIT_SAMP], // note_on v1; wait_samp
        &[END],
    ]
    .concat();
    let add: Vec<u8> = [&[4, 7, 1][..], &[16, 6, 7], &[END]].concat();
    let stop: &[u8] = &[END];
    let mut player = Player::new(vec![module(&[&main, &add, stop], 1)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert_eq!(out[0], 1, "the note played");

    // Start at 5 instead: 6 != 4, so no note.
    let main5: Vec<u8> = [&[4, 6, 5][..], &main[3..]].concat();
    let mut player = Player::new(vec![module(&[&main5, &add, stop], 1)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert!(out.iter().all(|&s| s == 0) && player.is_idle());
}

#[test]
fn waits_count_milliseconds_and_loops_wait_a_block() {
    // wait 10 ms, then play, then loop back to the start.
    let routine: Vec<u8> = [&[4, 6, 10][..], &[9, 6], &[2, 5, WAIT_SAMP], &[32], &[END]].concat();
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render(2000, &NoSources);
    // 10 ms is 220.5 frames: the thread wakes at the first block start
    // at or after frame 221.
    let first = out.iter().position(|&s| s != 0).unwrap();
    assert_eq!(first, 256);
    let second = out[first + 100..].iter().position(|&s| s != 0).unwrap() + first + 100;
    assert!(
        second > first + 100 + 220,
        "the loop waited again: {second}"
    );
    assert!(!player.is_idle());
    player.handle(&SoundEvent::StopObject { object: 9 }, &NoSources);
    assert!(player.is_idle());
}

#[test]
fn hit_lists_choose_with_the_seed() {
    let choose: Vec<u8> = [
        &[5, 5][..],
        &long(5),    // loadl v1, 5
        &[68, 5],    // smart_setlist v1
        &[39, 5],    // smart_choose v1
        &[6, 50, 5], // set patch, v1
        &[2, 5, WAIT_SAMP, END],
    ]
    .concat();
    let first_frames: Vec<i16> = (1..=12)
        .map(|seed| {
            let mut player = Player::new(vec![module(&[&choose], 0)], samples(), seed);
            play(&mut player, "e0");
            player.render(1, &NoSources)[0]
        })
        .collect();
    // Patch 1 starts at 1, patch 2 at 2000: both get chosen, and a seed
    // always chooses the same.
    assert!(first_frames.contains(&1) && first_frames.contains(&2000));
    let again: Vec<i16> = (1..=12)
        .map(|seed| {
            let mut player = Player::new(vec![module(&[&choose], 0)], samples(), seed);
            play(&mut player, "e0");
            player.render(1, &NoSources)[0]
        })
        .collect();
    assert_eq!(first_frames, again);
}

struct Gender(i32);

impl Sources for Gender {
    fn field(&self, source: i32, field: i32) -> Option<i32> {
        assert_eq!((source, field), (9, 3));
        Some(self.0)
    }

    fn position(&self, _: i32) -> Option<[i32; 3]> {
        Some([1, 1, 1])
    }

    fn projection(&self, _: i32) -> Option<Projection> {
        Some(MIDDLE)
    }
}

#[test]
fn scripts_ask_the_simulation_about_their_object() {
    // v2 = field 3 of arg1 (the object); play only when it is 1.
    let routine: Vec<u8> = [
        &[4, 7, 3][..], // loadb v3, 3
        &[72, 6, 1, 7], // getsrcdatafield v2, arg1, v3
        &[4, 8, 1],     // loadb v4, 1
        &[20, 6, 8],    // cmp v2, v4
        &[63],
        &long(0x101), // ifne end
        &[2, 5, WAIT_SAMP],
        &[END],
    ]
    .concat();
    let stop: &[u8] = &[END];
    for (answer, plays) in [(1, true), (0, false)] {
        let mut player = Player::new(vec![module(&[&routine, stop], 1)], samples(), 1);
        play(&mut player, "e0");
        let out = player.render(4, &Gender(answer));
        assert_eq!(out[0] == 1, plays);
    }
}

#[test]
fn a_refused_field_leaves_the_register_and_is_a_fault() {
    // v2 = 1; v2 = field v3 of arg1; play only when v2 is still 1.
    let routine = |field: u8| -> Vec<u8> {
        [
            &[4, 6, 1][..], // loadb v2, 1
            &[4, 7, field], // loadb v3, field
            &[72, 6, 1, 7], // getsrcdatafield v2, arg1, v3
            &[4, 8, 1],     // loadb v4, 1
            &[20, 6, 8],    // cmp v2, v4
            &[63],
            &long(0x101), // ifne end
            &[2, 5, WAIT_SAMP],
            &[END],
        ]
        .concat()
    };
    let mut snapshot = Snapshot::default();
    snapshot.objects.insert(9, [5; sims_audio::sources::FIELDS]);
    snapshot.positions.insert(9, [1, 1, 1]);
    snapshot.screen.insert(9, MIDDLE);
    for (field, plays, faults) in [(21, true, 1), (20, false, 0)] {
        let code = routine(field);
        let mut player = Player::new(vec![module(&[&code, &[END]], 1)], samples(), 1);
        play(&mut player, "e0");
        let out = player.render(4, &snapshot);
        assert_eq!(out[0] == 1, plays, "field {field}");
        assert_eq!(player.faults.len(), faults, "{:?}", player.faults);
        if faults > 0 {
            assert!(player.faults[0].contains("Illegal source parameter request"));
        }
    }
}

#[test]
fn a_snapshot_answers_by_object_and_field() {
    let mut snapshot = Snapshot::default();
    // With nothing sent yet, no object is in a room.
    assert_eq!(snapshot.field(9, 13), Some(100));
    let mut answers = [0; sims_audio::sources::FIELDS];
    answers[3] = 7;
    snapshot.objects.insert(9, answers);
    assert_eq!(snapshot.field(9, 3), Some(7));
    assert_eq!(snapshot.field(9, 13), Some(0));
    assert_eq!(snapshot.field(10, 13), Some(100));
    assert_eq!(snapshot.field(-1, 13), Some(100));
    // A negative field falls to the query's default; 21 on is refused.
    assert_eq!(snapshot.field(9, -1), Some(0));
    assert_eq!(snapshot.field(9, i32::MIN), Some(0));
    assert_eq!(snapshot.field(9, 21), None);
    assert_eq!(snapshot.field(9, i32::MAX), None);
}

#[test]
fn a_pause_holds_a_playing_note_where_it_is() {
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic], 1)], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(
        player.render(40, &NoSources),
        (1..=40).collect::<Vec<i16>>()
    );
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(player.is_paused());
    // Held: silent, and the thread waits, however long.
    assert!(player.render(3 * BLOCK, &NoSources).iter().all(|&s| s == 0));
    assert!(!player.is_idle());
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert!(!player.is_paused());
    // It goes on from where it was.
    assert_eq!(
        player.render(60, &NoSources),
        (41..=100).collect::<Vec<i16>>()
    );
    player.render_until_idle(4 * BLOCK, &NoSources);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

/// Waits a second (loadl v1, 1000; wait v1), then plays.
fn waits_a_second() -> Vec<u8> {
    [
        &[LOADL, 5][..],
        &long(1000),
        &[WAIT, 5],
        &[2, 5, WAIT_SAMP, END],
    ]
    .concat()
}

#[test]
fn an_unpause_ends_a_wait_held_by_the_pause() {
    let routine = waits_a_second();
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    // Letting the track go moves its deadline to now (0x57dc8e): the
    // thread runs at the next block, long before its second is up.
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(
        player.render(10, &NoSources),
        (1..=10).collect::<Vec<i16>>()
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn an_unpause_without_a_pause_ends_every_wait() {
    let routine = waits_a_second();
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(
        player.render(10, &NoSources),
        (1..=10).collect::<Vec<i16>>()
    );
}

#[test]
fn a_pause_spares_the_interface_and_lets_new_sounds_play() {
    // Tracks 0x604 and 0x605 (`ui_click`, which a pause spares).
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let mut player = Player::new(
        vec![module_with(&[generic, generic], 1, 0x604, "")],
        samples(),
        1,
    );
    play(&mut player, "e0");
    play(&mut player, "e1");
    let both: Vec<i16> = (1..=10).map(|v| 2 * v).collect();
    assert_eq!(player.render(10, &NoSources), both);
    player.handle(&SoundEvent::Pause, &NoSources);
    assert_eq!(
        player.render(10, &NoSources),
        (11..=20).collect::<Vec<i16>>()
    );
    // Starting a held track whose script still runs lets it go on
    // (0x57da05): its note goes on at once and no second thread
    // starts, so nothing new joins at the next block (frame 64).
    play(&mut player, "e0");
    let out = player.render(100, &NoSources);
    let resumed: Vec<i16> = (0..100)
        .map(|i| {
            let held = if i < 90 { 11 + i } else { 0 };
            let click = if i < 80 { 21 + i } else { 0 };
            held + click
        })
        .collect();
    assert_eq!(out, resumed);
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

/// Records what the scripts ask (`getsrcdatafield`) and answers 0.
#[derive(Default)]
struct Asked(std::cell::RefCell<Vec<(i32, i32)>>);

impl Sources for Asked {
    fn field(&self, source: i32, field: i32) -> Option<i32> {
        self.0.borrow_mut().push((source, field));
        Some(0)
    }

    fn position(&self, _: i32) -> Option<[i32; 3]> {
        Some([1, 1, 1])
    }

    fn projection(&self, _: i32) -> Option<Projection> {
        Some(MIDDLE)
    }
}

#[test]
fn starting_a_held_track_gives_its_script_the_new_arguments_and_ends_its_wait() {
    // arg2 = 8, arg3 = 9; wait a second; ask field arg2 of arg1.
    let routine: Vec<u8> = [
        &[4, 2, 8][..], // loadb arg2, 8
        &[4, 3, 9],     // loadb arg3, 9
        &[LOADL, 5],
        &long(1000), // loadl v1, 1000
        &[WAIT, 5],
        &[72, 6, 1, 2], // getsrcdatafield v2, arg1, arg2
        &[END],
    ]
    .concat();
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    let asked = Asked::default();
    play(&mut player, "e0");
    player.render(BLOCK, &asked);
    player.handle(&SoundEvent::Pause, &NoSources);
    // Started again for object 4 while held: the same script goes on,
    // with arguments 4, 0 and 0 (0x57d9cb to 0x57d9eb), at the next
    // block, its second's wait cut short (0x57dc8e).
    play_for(&mut player, 4, "e0");
    player.render(BLOCK, &asked);
    assert_eq!(*asked.0.borrow(), [(4, 0)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn starting_a_held_track_needs_no_free_thread() {
    let routine = waits_a_second();
    let routines = vec![&routine[..]; sims_audio::player::THREADS];
    let mut player = Player::new(vec![module(&routines, 1)], samples(), 1);
    for i in 0..sims_audio::player::THREADS {
        play(&mut player, &format!("e{i}"));
    }
    player.render(BLOCK, &NoSources);
    player.handle(&SoundEvent::Pause, &NoSources);
    // Every thread is taken, but the held e0 is let go, not started.
    play(&mut player, "e0");
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert_eq!(
        player.render(10, &NoSources),
        (1..=10).collect::<Vec<i16>>()
    );
}

#[test]
fn a_held_fading_track_started_again_resumes_and_cancels_its_fade() {
    // `note_on v1; end` followed by `loadl 5, 1000; wait 5`: `end` while
    // the note plays starts a fade-out at speed 100 and keeps the track
    // running (0x57e3ee → 0x57edc8 → 0x57dca3). Pausing holds it;
    // starting it again while paused clears both the hold and the fade
    // (0x57d9b2..0x57da12) and resumes the single note without starting
    // a second one.
    let note_and_end: &[u8] = &[2, 5, END];
    let mut hold_pc = vec![SET_LOOP];
    hold_pc.extend(loadl(5, 1000));
    hold_pc.extend([WAIT, 5, LOOP]);
    let mut player = Player::new(vec![module(&[note_and_end, &hold_pc], 1)], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(
        player.render(10, &NoSources),
        (1..=10).collect::<Vec<i16>>()
    );
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(player.render(10, &NoSources).iter().all(|&s| s == 0));
    play(&mut player, "e0");
    let out = player.render(50, &NoSources);
    assert_eq!(out, (11..=60).collect::<Vec<i16>>());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kinds_21_and_22_pause_and_unpause() {
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let module = module_with(&[generic], 1, 10, "pause=21,0\nunpause=22,0\n");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e0");
    player.render(10, &NoSources);
    play(&mut player, "pause");
    assert!(player.is_paused());
    assert!(player.render(10, &NoSources).iter().all(|&s| s == 0));
    play(&mut player, "unpause");
    assert!(!player.is_paused());
    assert_eq!(player.render(5, &NoSources), [11, 12, 13, 14, 15]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn unknown_events_and_bad_code_are_faults_not_panics() {
    let bad: &[u8] = &[26];
    let spin: &[u8] = &[13, 1, 1, 0, 0]; // jump 0x101: itself, forever
    let mut player = Player::new(vec![module(&[bad, spin], 1)], samples(), 1);
    play(&mut player, "nothing");
    play(&mut player, "other");
    play(&mut player, "e0");
    play(&mut player, "e1");
    player.render(BLOCK, &NoSources);
    assert_eq!(player.faults.len(), 4, "{:?}", player.faults);
    assert!(player.faults[0].contains("no event nothing"));
    assert!(player.faults[1].contains("kind 7 is not one"));
    assert!(player.faults[2].contains("unknown opcode 26"));
    assert!(player.faults[3].contains("ran too long"));
    assert!(player.is_idle());
    player.handle(&SoundEvent::StopAll, &NoSources);
}

const LOADB_V1: u8 = 4;
const PLAY_TRK: u8 = 50;

#[test]
fn a_track_that_restarts_itself_runs_once_per_block() {
    // Track 10: interrupt = 1; v1 = 10; play_trk v1; then a note it
    // never reaches. Each run starts itself anew: from the top, in the
    // next block, so `render` returns and the note never plays.
    let respawn: &[u8] = &[
        LOADB_V1, 55, 1, LOADB_V1, 5, 10, PLAY_TRK, 5, 2, 5, WAIT_SAMP, END,
    ];
    let mut player = Player::new(vec![module(&[respawn], 1)], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(1, &NoSources), [0]);
    let out = player.render(20 * BLOCK, &NoSources);
    assert!(out.iter().all(|&s| s == 0));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert!(!player.is_idle());
    player.handle(&SoundEvent::StopAll, &NoSources);
    assert!(player.is_idle());
}

#[test]
fn a_track_that_plays_itself_without_interrupt_goes_on() {
    // Track 10: v1 = 10; play_trk v1; end. It runs, so the play starts
    // nothing, and the script goes on to its end.
    let respawn: &[u8] = &[LOADB_V1, 5, 10, PLAY_TRK, 5, END];
    let mut player = Player::new(vec![module(&[respawn], 0)], samples(), 1);
    play(&mut player, "e0");
    player.render(BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert!(player.is_idle());
}

#[test]
fn a_running_track_played_again_starts_no_second_thread() {
    // Track 10 plays patch 1 over and over, as the fountain does. Played
    // again while it runs, it neither starts again nor doubles: the ramp
    // goes on from where it was.
    let mut player = Player::new(vec![module(&[&LOOPING], 1)], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(
        player.render(10, &NoSources),
        (1..=10).collect::<Vec<i16>>()
    );
    for _ in 0..1000 {
        play(&mut player, "e0");
    }
    assert_eq!(
        player.render(10, &NoSources),
        (11..=20).collect::<Vec<i16>>()
    );
    for _ in 0..300 {
        play(&mut player, "e0");
        player.render(BLOCK, &NoSources);
    }
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    player.handle(&SoundEvent::StopObject { object: 9 }, &NoSources);
    player.render(12 * BLOCK, &NoSources);
    assert!(player.is_idle());
}

#[test]
fn a_play_of_a_running_track_ends_its_wait() {
    // Track 10 waits a second, then plays patch 1. Played again while it
    // waits, it is let go: its wait ends at the next block.
    let mut routine = loadl(6, 1000);
    routine.extend([WAIT, 6]);
    routine.extend(GENERIC);
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    assert!(player.render(2 * BLOCK, &NoSources).iter().all(|&s| s == 0));
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
}

#[test]
fn a_track_with_interrupt_set_starts_again_when_played() {
    // Track 10 sets interrupt (55), then plays patch 1 to its end. Played
    // again while it runs, it starts anew: its note is cut and the patch
    // plays from the start, once.
    let mut routine = loadb(55, 1).to_vec();
    routine.extend(GENERIC);
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(
        player.render(10, &NoSources),
        (1..=10).collect::<Vec<i16>>()
    );
    player.render(BLOCK - 10, &NoSources);
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_fresh_start_stops_the_note_kind_53_played_on_the_idle_track() {
    // seq plays patch 2 on track 10 while no thread of it runs; starting
    // track 10, which waits without a note, stops that note.
    let waiting: &[u8] = &[SET_LOOP, LOOP];
    let module = module_with(&[waiting], 1, 10, "seq=53,10,0,6\n");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "seq");
    assert_eq!(player.render(1, &NoSources), [2000]);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [0, 0]);
}

#[test]
fn threads_stop_at_the_thread_limit() {
    // A track has one thread at most, so it takes more than 256 tracks
    // that run for ever. The tracks past the limit do not start.
    let forever: &[u8] = &[LOOP];
    let routines = vec![forever; 300];
    let mut player = Player::new(vec![module(&routines, 0)], samples(), 1);
    for i in 0..300 {
        play(&mut player, &format!("e{i}"));
    }
    player.render(2 * BLOCK, &NoSources);
    assert!(
        player
            .faults
            .iter()
            .all(|f| f.contains("more than 256 threads")),
        "{:?}",
        player.faults
    );
    assert!(!player.faults.is_empty());
    assert!(player.faults.len() <= sims_audio::player::FAULTS);
}

#[test]
fn all_threads_share_one_instruction_budget_per_block() {
    // Twelve threads that jump to themselves forever. Ten use up the
    // block's 100,000 instructions and end as stuck; the other two wait
    // for the next block, where they end the same way.
    // Routine i jumps to track data 0x100 + i: itself.
    let spins: Vec<[u8; 5]> = (0..12).map(|i| [13, i, 1, 0, 0]).collect();
    let routines: Vec<&[u8]> = spins.iter().map(|s| &s[..]).collect();
    let mut player = Player::new(vec![module(&routines, 0)], samples(), 1);
    for i in 0..12 {
        play(&mut player, &format!("e{i}"));
    }
    player.render(1, &NoSources);
    assert_eq!(player.faults.len(), 11, "{:?}", player.faults);
    assert!(player.faults[10].contains("100000 instructions in one block"));
    assert!(!player.is_idle());
    player.render(BLOCK, &NoSources);
    let stuck = player.faults.iter().filter(|f| f.contains("ran too long"));
    assert_eq!(stuck.count(), 12);
    assert!(player.is_idle());
}

#[test]
fn interpolating_between_full_scale_samples_does_not_overflow() {
    let mut map = samples();
    map.insert(
        "one.xa".into(),
        Pcm {
            rate: 16_000,
            channels: 1,
            samples: vec![i16::MIN, i16::MAX],
        },
    );
    // Pitch 2400 plays at half speed: frame 1 is halfway.
    let mut routine = vec![LOADL, 21];
    routine.extend(long(2400));
    routine.extend([2, 5, WAIT_SAMP, END]);
    let mut player = Player::new(vec![module(&[&routine], 1)], map, 1);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [i16::MIN, -1]);
}

#[test]
fn unported_instructions_are_faults() {
    // test1 to test4 take no operand; seqgroup_wait takes one register,
    // the group instructions three. A seqgroup instruction whose register
    // is 0 is an error, as in the original.
    let cases: [(&[u8], &str); 8] = [
        (&[58], "test1 is not supported"),
        (&[59], "test2 is not supported"),
        (&[60], "test3 is not supported"),
        (&[61], "test4 is not supported"),
        (&[70, 5], "seqgroup_wait is not supported"),
        (&[29, 5, 6, 7], "add_to_group is not supported"),
        (&[30, 5, 6, 7], "remove_from_group is not supported"),
        (&[SEQGROUP_KILL, 0], "seqgroup_kill: bad register"),
    ];
    for (code, message) in cases {
        let mut routine = code.to_vec();
        routine.extend([NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, END]);
        let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
        play(&mut player, "e0");
        let out = player.render(4, &NoSources);
        assert_eq!(player.faults.len(), 1, "{message}: {:?}", player.faults);
        assert!(player.faults[0].contains(message), "{:?}", player.faults);
        // The thread ended before its note.
        assert_eq!(out, [0; 4]);
        assert!(player.is_idle());
    }
}

const SEQGROUP_KILL: u8 = 69;
const SEQGROUP_RETURN: u8 = 71;
const SEQGROUP_TRKID: u8 = 73;
const WAITEQ: u8 = 77;
const TEST: u8 = 14;
const IFEQ: u8 = 62;
/// `loadb r, value`.
fn loadb(register: u8, value: u8) -> [u8; 3] {
    [LOADB_V1, register, value]
}

#[test]
fn seqgroup_return_ends_its_thread_when_another_holds_the_group() {
    // Claim group 1 (register 5 holds it), then play patch 1.
    let mut routine = loadb(5, 1).to_vec();
    routine.extend([SEQGROUP_RETURN, 5, 2, 6, WAIT_SAMP, END]);
    let mut player = Player::new(vec![module(&[&routine, &routine], 1)], samples(), 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // e1's thread found e0's track holding the group and ended: the
    // sample plays once, not twice as loud.
    assert_eq!(&out[..100], &(1..=100).collect::<Vec<i16>>());
    // The slot keeps e1's track after its thread ended, so e0 now ends
    // too, until StopAll clears the slots.
    play(&mut player, "e0");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert!(out.iter().all(|&s| s == 0));
    player.handle(&SoundEvent::StopAll, &NoSources);
    play(&mut player, "e0");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    // From the next block.
    assert_eq!(&out[BLOCK..BLOCK + 100], &(1..=100).collect::<Vec<i16>>());
    // A second play of e1 while it runs starts nothing, so it does not
    // meet its own group: the sample plays once.
    player.handle(&SoundEvent::StopAll, &NoSources);
    play(&mut player, "e1");
    play(&mut player, "e1");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert_eq!(out.iter().find(|&&s| s != 0), Some(&1));
}

#[test]
fn seqgroup_kill_stops_a_holder_whose_extvol_is_0_at_once() {
    // e0 sets its extvol (19) to 0, claims group 1 and runs forever.
    let mut holder = loadb(19, 0).to_vec();
    holder.extend(loadb(5, 1));
    holder.extend([SEQGROUP_RETURN, 5, SET_LOOP, LOOP]);
    // e1 takes group 1, then plays only if the group has a holder (which
    // it should: itself), using patch 2.
    let mut killer = loadb(5, 1).to_vec();
    killer.extend([SEQGROUP_KILL, 5, SEQGROUP_TRKID, 5, 9, TEST, 9, IFEQ]);
    killer.extend(long(0x102));
    killer.extend(loadb(50, 2));
    killer.extend([2, 6, WAIT_SAMP, END]);
    let done: &[u8] = &[END];
    let mut player = Player::new(vec![module(&[&holder, &killer, done], 1)], samples(), 1);
    play(&mut player, "e0");
    let mut out = player.render(32, &NoSources);
    play(&mut player, "e1");
    out.extend(player.render_until_idle(10 * BLOCK, &NoSources));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // e1 ended e0 in the next block and played patch 2 (two.xa's frames
    // at 22,050 Hz); then nothing was left running.
    assert!(out[..BLOCK].iter().all(|&s| s == 0));
    assert_eq!(&out[BLOCK..BLOCK + 3], [2000, 4000, -2000]);
    assert!(player.is_idle());
}

#[test]
fn seqgroup_kill_fades_out_the_holder_then_runs_on() {
    // e0 claims group 1, waits until its extvol (19) is 1 (register 8
    // here), then plays patch 1.
    let mut holder = loadb(5, 1).to_vec();
    holder.extend([SEQGROUP_RETURN, 5]);
    holder.extend(loadb(8, 1));
    holder.extend([WAITEQ, 19, 8, 2, 6, WAIT_SAMP, END]);
    // e1 takes group 1 and plays patch 2.
    let mut killer = loadb(5, 1).to_vec();
    killer.extend([SEQGROUP_KILL, 5]);
    killer.extend(loadb(50, 2));
    killer.extend([2, 6, WAIT_SAMP, END]);
    let mut player = Player::new(vec![module(&[&holder, &killer], 1)], samples(), 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    let out = player.render_until_idle(20 * BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // e1 waits 2 ms, so it plays from the second block. e0's extvol falls
    // by 200 a block from 1024 to fade_dest, which starts at 0, passing
    // over 1. e1's second try (0x57eef3) finds the group its own and runs
    // on, so nothing ends e0: it stays, silent, waiting for an extvol it
    // has passed.
    assert!(out[..BLOCK].iter().all(|&s| s == 0));
    assert_eq!(&out[BLOCK..BLOCK + 3], [2000, 4000, -2000]);
    assert!(out[BLOCK + 3..].iter().all(|&s| s == 0));
    assert!(!player.is_idle());
}

#[test]
fn the_first_module_defining_an_event_wins_and_play_from_picks_one() {
    // The modules' events play tracks 10 and 20.
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let first = module(&[generic], 1);
    let second = module_with(&[generic], 2, 20, "");
    let mut player = Player::new(vec![first, second], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    player.handle(&SoundEvent::StopAll, &NoSources);
    player.play_from(1, 9, "e0", &NoSources);
    player.render(BLOCK - 3, &NoSources);
    assert_eq!(player.render(3, &NoSources), [2000, 4000, -2000]);
    player.handle(&SoundEvent::StopAll, &NoSources);
    player.play_from(2, 9, "e0", &NoSources);
    assert_eq!(player.faults, ["no event e0"]);
}

const ADD: u8 = 16;
const WAIT: u8 = 9;
const CALL: u8 = 7;
const LOADL: u8 = 5;
const SET_LOOP: u8 = 33;
const LOOP: u8 = 32;
const NOP: u8 = 15;

#[test]
fn sequence_groups_stop_at_the_group_limit() {
    // Claim a new group every block: 1, 2, 3, ...
    let mut routine = loadb(6, 1).to_vec();
    routine.extend([SET_LOOP, ADD, 5, 6, SEQGROUP_RETURN, 5, LOOP]);
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    player.render(300 * BLOCK, &NoSources);
    assert_eq!(player.faults.len(), 1, "{:?}", player.faults);
    assert!(player.faults[0].contains("more than 256 sequence groups"));
    assert!(player.is_idle());
}

#[test]
fn calls_across_waits_stop_at_the_depth_limit() {
    // wait 0 ms (register 7), then call this routine again.
    let mut routine = vec![WAIT, 7, CALL];
    routine.extend(long(0x100));
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    player.render(100 * BLOCK, &NoSources);
    assert_eq!(player.faults.len(), 1, "{:?}", player.faults);
    assert!(player.faults[0].contains("calls nested deeper than 64"));
    assert!(player.is_idle());
}

#[test]
fn a_missing_patch_is_not_cached() {
    // Each block, play patch 77, which no table defines: each attempt is
    // looked up (and reported) again, so nothing is kept for it.
    let mut routine = loadb(50, 77).to_vec();
    routine.extend([SET_LOOP, 2, 6, LOOP]);
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    player.render(3 * BLOCK, &NoSources);
    assert_eq!(player.faults, ["no patch 77"; 3]);
}

#[test]
fn threads_a_full_block_does_not_reach_run_first_in_the_next() {
    // Ten threads that each spend 10,000 instructions a block use the
    // whole block budget; an eleventh still gets to play its note.
    let mut busy = vec![NOP; 9_999];
    busy.push(LOOP);
    let generic: &[u8] = &[NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, END];
    let mut routines: Vec<&[u8]> = vec![&busy; 10];
    routines.push(generic);
    let mut player = Player::new(vec![module(&routines, 1)], samples(), 1);
    for i in 0..10 {
        play(&mut player, &format!("e{i}"));
    }
    play(&mut player, "e10");
    let out = player.render(3 * BLOCK, &NoSources);
    assert_eq!(&out[BLOCK..BLOCK + 3], [1, 2, 3]);
}

#[test]
fn a_fade_near_the_largest_value_stops_at_its_destination() {
    // Fade register 7 from i32::MAX - 7 to i32::MAX at speed 200, then
    // play patch 1 once it gets there.
    let mut routine = vec![LOADL, 7];
    routine.extend(long(i32::MAX as u32 - 7));
    routine.extend([LOADL, 40]);
    routine.extend(long(i32::MAX as u32));
    routine.extend([LOADL, 8]);
    routine.extend(long(i32::MAX as u32));
    routine.extend([LOADL, 42]);
    routine.extend(long(200));
    routine.extend(loadb(41, 7));
    routine.extend(loadb(45, 1));
    routine.extend([WAITEQ, 7, 8, 2, 6, WAIT_SAMP, END]);
    let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render(3 * BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert_eq!(&out[BLOCK..BLOCK + 3], [1, 2, 3]);
}

const IFNE: u8 = 63;

/// The test samples with patch 1 replaced by `frames` frames of 10,000.
fn loud(frames: usize) -> BTreeMap<String, Pcm> {
    let mut map = samples();
    map.insert(
        "one.xa".into(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![10_000; frames],
        },
    );
    map
}

/// `loadl r, value`.
fn loadl(register: u8, value: i32) -> Vec<u8> {
    let mut code = vec![LOADL, register];
    code.extend(long(value as u32));
    code
}

const GENERIC: [u8; 4] = [2, 5, WAIT_SAMP, END];

#[test]
fn vol_pan_group_and_master_volumes_scale_notes() {
    // vol 512 of 1024: -20 dB, a tenth (6553 / 65536).
    let mut routine = loadl(18, 512);
    routine.extend(GENERIC);
    let mut player = Player::new(vec![module(&[&routine], 1)], loud(4), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [999, 999]);
    // pan 0: all left.
    let mut routine = loadl(20, 0);
    routine.extend(GENERIC);
    let mut player = Player::new(vec![module(&[&routine], 1)], loud(4), 1);
    play(&mut player, "e0");
    assert_eq!(player.render_stereo(2, &NoSources), [10_000, 0, 10_000, 0]);
    // The group's volume (the test tracks are group 0) and the master
    // volume scale it the same way.
    let mut player = Player::new(vec![module(&[&GENERIC], 1)], loud(4), 1);
    player.set_group_volume(0, 512);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [999, 999]);
    let mut player = Player::new(vec![module(&[&GENERIC], 1)], loud(4), 1);
    player.set_master_volume(512);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [999, 999]);
}

#[test]
fn main_duckpri_halves_tracks_below_it_until_its_thread_ends() {
    // e0 asks for main_duckpri 30 and waits for ever; e1 (duck priority
    // 0) plays.
    let mut ducker = loadl(123, 30);
    ducker.extend([SET_LOOP, LOOP]);
    let mut player = Player::new(vec![module(&[&ducker, &GENERIC], 1)], loud(4), 1);
    player.handle(
        &SoundEvent::Play {
            object: 1,
            name: "e0".into(),
        },
        &NoSources,
    );
    play(&mut player, "e1");
    assert_eq!(player.render(2, &NoSources), [999, 999]);
    player.render(BLOCK - 2, &NoSources);
    // When the ducking thread stops, its request goes with it. (e1's
    // thread ends in the next block, so a play after that starts it
    // anew.)
    player.handle(&SoundEvent::StopObject { object: 1 }, &NoSources);
    player.render(BLOCK, &NoSources);
    play(&mut player, "e1");
    assert_eq!(player.render(2, &NoSources), [10_000, 10_000]);
}

#[test]
fn target_sends_register_writes_to_another_track() {
    // e1 sets track 10's (e0's) vol to 512 through target, then restores
    // its own target. Both start in the same block, e0 first.
    let mut setter = loadl(53, 10);
    setter.extend(loadl(18, 512));
    setter.extend(loadl(53, 0));
    setter.extend(loadl(18, 7));
    setter.push(END);
    let mut player = Player::new(vec![module(&[&GENERIC, &setter], 1)], loud(1000), 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    assert_eq!(player.render(2, &NoSources), [999, 999]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn every_register_write_sets_the_flags() {
    // loadb 5, 1 then ifne: the write leaves "not equal", so the branch
    // to routine 1 (a note) is taken.
    let mut routine = loadb(5, 1).to_vec();
    routine.push(IFNE);
    routine.extend(long(0x101));
    routine.push(END);
    let mut player = Player::new(vec![module(&[&routine, &GENERIC], 1)], loud(4), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [10_000, 10_000]);
}

#[test]
fn a_target_naming_no_running_track_is_the_thread_itself() {
    // Track 99 has no thread, so `target` stays here (the original's
    // lookup returns null, which it takes as itself): vol 0 is this
    // thread's, and its note is silent.
    let mut routine = loadl(53, 99);
    routine.extend(loadl(18, 0));
    routine.extend(GENERIC);
    let mut player = Player::new(vec![module(&[&routine], 1)], loud(4), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [0, 0]);
}

#[test]
fn target_follows_the_target_s_own_target() {
    // e0 targets track 11 (e1, a note) and waits; e2 targets track 10
    // (e0) and sets vol 512, which goes on to e1.
    let mut middle = loadl(53, 11);
    middle.extend([SET_LOOP, LOOP]);
    let mut setter = loadl(53, 10);
    setter.extend(loadl(18, 512));
    setter.push(END);
    let mut player = Player::new(
        vec![module(&[&middle, &GENERIC, &setter], 1)],
        loud(1000),
        1,
    );
    for event in ["e0", "e1", "e2"] {
        play(&mut player, event);
    }
    assert_eq!(player.render(2, &NoSources), [999, 999]);
}

#[test]
fn a_ducking_request_goes_as_soon_as_its_thread_ends() {
    // e0 asks for main_duckpri 30 and ends; e1, later in the same block,
    // finds it 0 and so ends without playing e2's note.
    let mut ducker = loadl(123, 30);
    ducker.push(END);
    let mut checker = vec![TEST, 123, IFNE];
    checker.extend(long(0x102));
    checker.push(END);
    let mut player = Player::new(vec![module(&[&ducker, &checker, &GENERIC], 1)], loud(4), 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    assert_eq!(player.render(2, &NoSources), [0, 0]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_thread_s_last_writes_reach_its_notes_when_it_fades_at_end() {
    // `note_on`, then `vol 0`, then `end` (followed by an explicit wait):
    // the track stays running while fading out (0x57dca3), and `vol 0`
    // silences its note immediately without faulting.
    let mut routine = NOTE_ON_V1.to_vec();
    routine.extend(loadl(18, 0));
    routine.push(END);
    let mut hold_pc = loadl(5, 1000);
    hold_pc.extend([WAIT, 5]);
    let mut player = Player::new(vec![module(&[&routine, &hold_pc], 1)], loud(4), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [0, 0]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn too_many_frames_at_once_is_a_fault_not_a_panic() {
    let mut player = Player::new(vec![module(&[&GENERIC], 1)], loud(4), 1);
    assert!(player.render_stereo(usize::MAX, &NoSources).is_empty());
    assert!(player.render(usize::MAX / 2 + 1, &NoSources).is_empty());
    // More than an hour at once is refused too, whether or not it would
    // fit in memory.
    let over = sims_audio::player::RENDER_LIMIT + 1;
    assert!(player.render_stereo(over, &NoSources).is_empty());
    assert_eq!(player.faults.len(), 3, "{:?}", player.faults);
    // Rendering until idle stops at the limit.
    play(&mut player, "e0");
    let out = player.render_until_idle(usize::MAX, &NoSources);
    assert!(out.len() <= BLOCK);
}

/// Plays patch 1 over and over: `note_on v1; wait_samp; loop`.
const LOOPING: [u8; 4] = [2, 5, WAIT_SAMP, LOOP];

fn play_for(player: &mut Player<BTreeMap<String, Pcm>>, object: u32, name: &str) {
    player.handle(
        &SoundEvent::Play {
            object,
            name: name.into(),
        },
        &NoSources,
    );
}

#[test]
fn a_fade_event_fades_a_track_and_may_end_it() {
    // kind 18 on track 10: the first argument is the speed, the caller's
    // object when the event gives none, and 100 when that is 0 too; the
    // third argument ends the track when the fade ends.
    let events = "keep=18,10\nkill=18,10,0,0,1\nfast=18,10,512\n";
    for (event, blocks) in [("keep", None), ("kill", Some(11)), ("fast", None)] {
        let module = module_with(&[&LOOPING], 1, 10, events);
        let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
        play(&mut player, "e0");
        player.render(BLOCK, &NoSources);
        play_for(&mut player, 0, event);
        // extvol falls by the speed each block, from 1024.
        let mut ended = None;
        for block in 1..=20 {
            player.render(BLOCK, &NoSources);
            if player.is_idle() && ended.is_none() {
                ended = Some(block);
            }
        }
        assert!(player.faults.is_empty(), "{:?}", player.faults);
        assert_eq!(ended, blocks, "{event}");
        if blocks.is_none() {
            // Faded to silence, still running.
            assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
        }
    }
}

#[test]
fn a_fade_event_s_speed_is_the_object_when_it_gives_none() {
    let module = module_with(&[&LOOPING], 1, 10, "kill=18,10,0,0,1\n");
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play(&mut player, "e0");
    player.render(BLOCK, &NoSources);
    // Object 512: two blocks to 0.
    play_for(&mut player, 512, "kill");
    player.render(2 * BLOCK, &NoSources);
    assert!(!player.is_idle());
    player.render(BLOCK, &NoSources);
    assert!(player.is_idle());
}

#[test]
fn a_fade_event_for_a_track_not_running_does_nothing() {
    let module = module_with(&[&LOOPING], 1, 10, "kill=18,10,0,0,1\n");
    let mut player = Player::new(vec![module], loud(4), 1);
    play(&mut player, "kill");
    player.render(BLOCK, &NoSources);
    assert!(player.faults.is_empty() && player.is_idle());
}

/// Object 1 is on the left of the screen, object 2 on the right.
struct TwoSides;

impl Sources for TwoSides {
    fn field(&self, _: i32, _: i32) -> Option<i32> {
        Some(0)
    }

    fn position(&self, object: i32) -> Option<[i32; 3]> {
        matches!(object, 1 | 2).then_some([40, 56, 1])
    }

    fn projection(&self, object: i32) -> Option<Projection> {
        let x = match object {
            1 => 100,
            2 => 700,
            _ => return None,
        };
        Some(Projection {
            at: [x, 300],
            screen: [800, 600],
            zoom: 1,
            shown_level: 1,
        })
    }
}

#[test]
fn an_object_that_goes_fades_out_its_instances() {
    // Objects 1 and 2 play track 10: one track, played once. Object 1
    // goes and the track plays on for object 2, placed again: on the
    // right.
    let module = module_with(&[&LOOPING], 1, 10, "");
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    let sides = TwoSides;
    for object in [1, 2] {
        player.handle(
            &SoundEvent::Play {
                object,
                name: "e0".into(),
            },
            &sides,
        );
    }
    let both = player.render_stereo(1, &sides);
    player.render(BLOCK - 1, &sides);
    player.handle(&SoundEvent::StopObject { object: 1 }, &sides);
    let right = player.render_stereo(1, &sides);
    assert!(right[1] > right[0], "{right:?}");
    assert!(
        right[1] - right[0] > both[1] - both[0],
        "{both:?} {right:?}"
    );
    player.render(BLOCK - 1, &sides);
    // The last object going fades the track by 100 a block, from 1024,
    // and ends it.
    player.handle(&SoundEvent::StopObject { object: 2 }, &sides);
    let first = player.render(1, &sides)[0];
    assert!(first > 0 && first < 10_000, "{first}");
    player.render(12 * BLOCK, &sides);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn an_object_that_goes_ends_its_silent_threads_at_once() {
    // e0 waits for ever without a note.
    let waiting: &[u8] = &[SET_LOOP, LOOP];
    let module = module_with(&[waiting], 1, 10, "");
    let mut player = Player::new(vec![module], samples(), 1);
    play_for(&mut player, 1, "e0");
    player.render(BLOCK, &NoSources);
    player.handle(&SoundEvent::StopObject { object: 1 }, &NoSources);
    assert!(player.is_idle());
}

#[test]
fn turning_on_a_tv_lets_go_of_what_it_had_playing() {
    // kind 30 on tracks 10 and 11 for object 3, given as the event's
    // first argument.
    let events = "on0=30,10,3\non1=30,11,3\noff=31,10\n";
    let module = module_with(&[&LOOPING, &LOOPING], 1, 10, events);
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play(&mut player, "on0");
    assert_eq!(player.render(1, &NoSources), [10_000]);
    player.render(BLOCK - 1, &NoSources);
    // kTurnOffTv does nothing (the scripts turn sets off themselves).
    play(&mut player, "off");
    assert_eq!(player.render(1, &NoSources), [10_000]);
    player.render(BLOCK - 1, &NoSources);
    // A new channel: track 10 fades out while track 11 plays.
    play(&mut player, "on1");
    let both = player.render(1, &NoSources)[0];
    assert!(both > 10_000 && both < 20_000, "{both}");
    player.render(12 * BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [10_000]);
    // Object 3 going ends track 11 too.
    player.handle(&SoundEvent::StopObject { object: 3 }, &NoSources);
    player.render(12 * BLOCK, &NoSources);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_sequenced_patch_plays_right_after_the_track_s_note() {
    // kind 53 on track 10 with hit list 6 (patch 2 alone).
    let module = module_with(&[&GENERIC], 1, 10, "seq=53,10,0,6\n");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e0");
    player.render(10, &NoSources);
    play(&mut player, "seq");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // One.xa's last 90 frames, then two.xa with no gap, as one note the
    // thread waits for.
    let mut expected: Vec<i16> = (11..=100).collect();
    expected.extend([2000, 4000, -2000]);
    assert_eq!(&out[..93], &expected);
    assert!(out[93..].iter().all(|&s| s == 0));
}

#[test]
fn a_sequenced_patch_with_no_note_playing_starts_one() {
    // e0 waits for ever with no note; with no thread at all the patch
    // plays alone.
    let waiting: &[u8] = &[SET_LOOP, LOOP];
    for run in [true, false] {
        let module = module_with(&[waiting], 1, 10, "seq=53,10,0,6\n");
        let mut player = Player::new(vec![module], samples(), 1);
        if run {
            play(&mut player, "e0");
            player.render(BLOCK, &NoSources);
        }
        play(&mut player, "seq");
        assert_eq!(player.render(4, &NoSources), [2000, 4000, -2000, 0]);
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn radio_next_song_stops_each_station_s_note() {
    // Tracks 260 (0x104) and 261; kind 19 stops 260's note, so its
    // script goes on (here, to the end). 261 is not a radio track.
    let module = module_with(&[&GENERIC, &GENERIC], 1, 260, "next=19,0\n");
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    player.render(BLOCK, &NoSources);
    play(&mut player, "next");
    assert_eq!(player.render(1, &NoSources), [10_000]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kill_all_and_kill_object_sounds_events() {
    let events = "all=20,0\nobjects=105,0\non=30,10\n";
    let module = module_with(&[&LOOPING], 1, 10, events);
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play_for(&mut player, 1, "e0");
    player.render(BLOCK, &NoSources);
    play(&mut player, "all");
    assert!(player.is_idle());
    // kind 105 fades out instances of objects above 0.
    let module = module_with(&[&LOOPING], 1, 10, events);
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play_for(&mut player, 1, "e0");
    player.render(BLOCK, &NoSources);
    play(&mut player, "objects");
    player.render(12 * BLOCK, &NoSources);
    assert!(player.is_idle());
    // Kind 1 starts nothing for an object of 0 (0x57375b). Kind 30
    // records it, but kind 105 leaves it: its track plays on.
    let module = module_with(&[&LOOPING], 1, 10, events);
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play_for(&mut player, 0, "e0");
    assert!(player.is_idle());
    play_for(&mut player, 0, "on");
    player.render(BLOCK, &NoSources);
    play(&mut player, "objects");
    player.render(12 * BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [10_000]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kinds_that_do_nothing_and_kinds_not_ported() {
    let events = "debug=44,0\nempty=29,10\nnone=1907,10\nreload=104,10\n";
    let module = module_with(&[&GENERIC], 1, 10, events);
    let mut player = Player::new(vec![module], samples(), 1);
    for name in ["debug", "empty", "none"] {
        play(&mut player, name);
    }
    assert!(player.faults.is_empty() && player.is_idle());
    play(&mut player, "reload");
    assert_eq!(player.faults.len(), 1);
    assert!(player.faults[0].contains("kind 104 is not one"));
}

/// Object 9 is on the screen right of the middle; every object's field 5
/// is 42. Records what is asked, as [`Asked`] does.
#[derive(Default)]
struct PianoRoom(std::cell::RefCell<Vec<(i32, i32)>>);

impl Sources for PianoRoom {
    fn field(&self, source: i32, field: i32) -> Option<i32> {
        self.0.borrow_mut().push((source, field));
        Some(if field == 5 { 42 } else { 0 })
    }

    fn position(&self, object: i32) -> Option<[i32; 3]> {
        (object == 9).then_some([40, 56, 1])
    }

    fn projection(&self, object: i32) -> Option<Projection> {
        (object == 9).then_some(Projection {
            at: [600, 300],
            screen: [800, 600],
            zoom: 1,
            shown_level: 1,
        })
    }
}

#[test]
fn the_piano_plays_where_its_object_is_and_sets_the_skill() {
    use sims_audio::player::{PIANO, PIANO_SKILL};
    // Both tracks ask about their first argument, as the fields their
    // `extvol` and `pan`: the piano every block, the skill once.
    let mut piano = vec![SET_LOOP];
    piano.extend([GETSRCDATAFIELD, 6, 1, 19, GETSRCDATAFIELD, 6, 1, 20, LOOP]);
    let skill = [GETSRCDATAFIELD, 6, 1, 19, GETSRCDATAFIELD, 6, 1, 20, END];
    let events = "piano=43,0\nunplaced=43,0,8,300,600\nkill9=23,9\nkill8=23,8\n";
    let module = module_of_tracks(&[(PIANO, &piano), (PIANO_SKILL, &skill)], events);
    let mut player = Player::new(vec![module], samples(), 1);
    let room = PianoRoom::default();
    let asked = || {
        let mut asked = std::mem::take(&mut *room.0.borrow_mut());
        asked.sort();
        asked
    };
    // Object 9 is placed with the piano's `Spl` (20): 464 at 527. The
    // skill (its field 5, asked at once) starts the skill track, whose
    // levels are its (0) second and third arguments.
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "piano".into(),
        },
        &room,
    );
    assert_eq!(asked(), [(9, 5)]);
    player.render(BLOCK, &room);
    assert_eq!(asked(), [(9, 464), (9, 527), (42, 0), (42, 0)]);
    // The piano still runs, so a start for object 8 (not on the lot)
    // starts no second piano: its arguments become 8 and the event's
    // levels, but its `extvol` and `pan` stay. The skill track ended, so
    // it starts anew.
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "unplaced".into(),
        },
        &room,
    );
    assert_eq!(asked(), [(8, 5)]);
    player.render(BLOCK, &room);
    assert_eq!(asked(), [(8, 464), (8, 527), (42, 0), (42, 0)]);
    // The piano's one instance is the object of its last fresh start, 9:
    // letting go of 8 leaves it playing, and of 9 ends it.
    play(&mut player, "kill8");
    player.render(12 * BLOCK, &room);
    assert!(!player.is_idle());
    play(&mut player, "kill9");
    player.render(12 * BLOCK, &room);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_piano_whose_number_is_a_patch_still_starts_the_skill_track() {
    use sims_audio::player::{PIANO, PIANO_SKILL};
    // The piano's number is registered as a patch. Its start does
    // nothing (0x57bf80), but the skill is still asked for and the skill
    // track started (0x573ec3 to 0x573ef2).
    let skill = [GETSRCDATAFIELD, 6, 1, 19, GETSRCDATAFIELD, 6, 1, 20, END];
    let events = format!("piano=43,0\n[Patch]\n{PIANO}=0,p,\"one.xa\"\n");
    let module = module_of_tracks(&[(PIANO_SKILL, &skill)], &events);
    let mut player = Player::new(vec![module], samples(), 1);
    let room = PianoRoom::default();
    let asked = || {
        let mut asked = std::mem::take(&mut *room.0.borrow_mut());
        asked.sort();
        asked
    };
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "piano".into(),
        },
        &room,
    );
    assert_eq!(asked(), [(9, 5)]);
    player.render(BLOCK, &room);
    assert_eq!(asked(), [(42, 0), (42, 0)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_piano_let_go_from_a_pause_keeps_its_object() {
    use sims_audio::player::{PIANO, PIANO_SKILL};
    let mut piano = vec![SET_LOOP];
    piano.extend([GETSRCDATAFIELD, 6, 1, 19, LOOP]);
    let skill = [END];
    let events = "piano=43,0\nunplaced=43,0,8,300,600\nkill9=23,9\nkill8=23,8\n";
    let module = module_of_tracks(&[(PIANO, &piano), (PIANO_SKILL, &skill)], events);
    let room = PianoRoom::default();
    let event = |name: &str, object| SoundEvent::Play {
        object,
        name: name.into(),
    };
    // The piano plays for 9, then is held by a pause; the start for 8
    // lets it go, and the piano's one instance is still 9.
    let mut player = Player::new(vec![module], samples(), 1);
    player.handle(&event("piano", 9), &room);
    player.render(BLOCK, &room);
    player.handle(&SoundEvent::Pause, &room);
    player.handle(&event("unplaced", 0), &room);
    player.handle(&SoundEvent::Unpause, &room);
    play(&mut player, "kill8");
    player.render(12 * BLOCK, &room);
    assert!(!player.is_idle());
    play(&mut player, "kill9");
    player.render(12 * BLOCK, &room);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_start_of_a_running_track_is_not_refused_at_the_thread_limit() {
    use sims_audio::player::{PIANO, THREADS};
    // The piano asks about its object every block; tracks 20 on loop.
    let mut piano = vec![SET_LOOP];
    piano.extend([GETSRCDATAFIELD, 6, 1, 19, LOOP]);
    let filler = [SET_LOOP, LOOP];
    let events = "piano=43,0\nunplaced=43,0,8,300,600\nkill9=23,9\nkill8=23,8\n";
    let mut tracks: Vec<(u32, &[u8])> = vec![(PIANO, &piano)];
    for track in 20..20 + THREADS as u32 {
        tracks.push((track, &filler));
    }
    let module = module_of_tracks(&tracks, events);
    let room = PianoRoom::default();
    let event = |name: &str, object| SoundEvent::Play {
        object,
        name: name.into(),
    };
    let mut player = Player::new(vec![module], samples(), 1);
    player.handle(&event("piano", 9), &room);
    for track in 20..20 + THREADS as u32 - 1 {
        play(&mut player, &format!("e{track}"));
    }
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // A fresh start is refused at the limit.
    play(&mut player, &format!("e{}", 20 + THREADS - 1));
    assert_eq!(player.faults.len(), 1);
    assert!(player.faults[0].contains("threads"));
    // The start for 8 makes no thread, so it is not refused: the piano
    // asks about 8 now, and its instance stays 9.
    player.handle(&event("unplaced", 0), &room);
    assert_eq!(player.faults.len(), 1, "{:?}", player.faults);
    room.0.take();
    player.render(BLOCK, &room);
    let asked = room.0.take();
    assert!(asked.iter().any(|&(object, _)| object == 8), "{asked:?}");
    assert!(asked.iter().all(|&(object, _)| object != 9), "{asked:?}");
    play(&mut player, "kill8");
    player.render(12 * BLOCK, &room);
    room.0.take();
    player.render(BLOCK, &room);
    assert!(room.0.take().iter().any(|&(object, _)| object == 8));
    play(&mut player, "kill9");
    player.render(12 * BLOCK, &room);
    room.0.take();
    player.render(BLOCK, &room);
    assert!(room.0.take().is_empty());
}

#[test]
fn the_piano_asks_the_skill_even_with_no_skill_track() {
    use sims_audio::player::PIANO;
    let module = module_of_tracks(&[(PIANO, &[END])], "piano=43,0\n");
    let mut player = Player::new(vec![module], samples(), 1);
    let room = PianoRoom::default();
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "piano".into(),
        },
        &room,
    );
    assert_eq!(room.0.take(), [(9, 5)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_fresh_start_sets_the_levels_from_the_arguments() {
    // Each track asks about its `extvol`, as the field its `pan`.
    let asker = [GETSRCDATAFIELD, 6, 19, 20, END];
    // Track 12 sets arg2 = 7, arg3 = 8 and plays track 11.
    let mut starter = [loadb(2, 7), loadb(3, 8), loadb(5, 11)].concat();
    starter.extend([PLAY_TRK, 5, END]);
    let module = module_of_kinds(
        &[(10, 1, &asker), (11, 2, &asker), (12, 2, &starter)],
        "levels=1,10,300,600\n",
    );
    let mut player = Player::new(vec![module], samples(), 1);
    // `kArgsVolPan`: the first two arguments.
    play(&mut player, "levels");
    assert_eq!(asked_in_a_block(&mut player), [(300, 600)]);
    // `kArgsIdVolPan`, started by `play_trk` for the next block: the
    // caller's second and third.
    play(&mut player, "e12");
    assert_eq!(asked_in_a_block(&mut player), []);
    assert_eq!(asked_in_a_block(&mut player), [(7, 8)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn the_piano_without_its_track_does_nothing() {
    let module = module_with(&[&GENERIC], 1, 10, "piano=43,10\n");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "piano");
    assert!(player.faults.is_empty() && player.is_idle());
}

const GETSRCDATAFIELD: u8 = 72;
const KILL_TRK: u8 = 51;

/// A module with each routine run by the track given with it, and event
/// `e{track}` (`kSndobPlay`) for each.
fn module_of_tracks(tracks: &[(u32, &[u8])], events: &str) -> Module {
    let tracks: Vec<(u32, u8, &[u8])> = tracks.iter().map(|&(t, r)| (t, 2, r)).collect();
    module_of_kinds(&tracks, events)
}

/// As [`module_of_tracks`], each track with its argument kind
/// (`kArgsVolPan` 1, `kArgsIdVolPan` 2).
fn module_of_kinds(tracks: &[(u32, u8, &[u8])], events: &str) -> Module {
    let routines: Vec<&[u8]> = tracks.iter().map(|&(_, _, routine)| routine).collect();
    let (hit, addresses) = assemble(&routines);
    let mut hot = String::from("[EventMappingEquate]\nkSndobPlay=1\n[TrackData]\n");
    for (i, address) in addresses.iter().enumerate() {
        hot.push_str(&format!("{:#x}={address:#x}\n", 0x100 + i));
    }
    hot.push_str("[Patch]\n1=0,one,\"one.xa\"\n[Track]\n");
    for (i, (track, arguments, _)) in tracks.iter().enumerate() {
        hot.push_str(&format!(
            "{track}=0,t{track},1024,{arguments},0,0,0,{:#x},1\n",
            0x100 + i
        ));
    }
    hot.push_str("[EventMapping]\n");
    for (track, _, _) in tracks {
        hot.push_str(&format!("e{track}=kSndobPlay,{track}\n"));
    }
    hot.push_str(events);
    Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap()
}

/// Asks the simulation about field `field` of the object its first
/// argument names (getsrcdatafield v2, arg1, v1), then ends, or, with
/// `again`, asks again every block.
fn asks(field: u32, again: bool) -> Vec<u8> {
    let mut routine = vec![LOADL, 5];
    routine.extend(long(field));
    if again {
        routine.push(SET_LOOP);
    }
    routine.extend([GETSRCDATAFIELD, 6, 1, 5]);
    routine.push(if again { LOOP } else { END });
    routine
}

/// Renders a block; returns what the scripts asked in it, sorted.
fn asked_in_a_block(player: &mut Player<BTreeMap<String, Pcm>>) -> Vec<(i32, i32)> {
    let asked = Asked::default();
    player.render(BLOCK, &asked);
    let mut asked = asked.0.into_inner();
    asked.sort();
    asked
}

#[test]
fn the_music_mode_starts_and_stops_the_music_tracks() {
    use sims_audio::player::music::*;
    // Each music track asks about its first argument, as the field the
    // track's own number: the ones that set the music once, the loops
    // (and the main music, which they start in the game) every block.
    let once = |t| (t, asks(t, false));
    let loops = |t| (t, asks(t, true));
    let tracks = [
        once(SET_MAIN_MUSIC),
        once(SET_MAIN_KILL_FIRST),
        once(FADE_AND_KILL_SLOW),
        loops(MAIN_MUSIC_LOOP),
        loops(RIVER_LOOP),
        loops(MT_RIVER_LOOP),
    ];
    let tracks: Vec<(u32, &[u8])> = tracks.iter().map(|(t, r)| (*t, &r[..])).collect();
    let events: String = (0..=18).map(|m| format!("m{m}=36,{m}\n")).collect();
    let module = module_of_tracks(&tracks, &events);
    let mut player = Player::new(vec![module], samples(), 1);
    let mode = |player: &mut Player<_>, m: u32| {
        play(player, &format!("m{m}"));
        asked_in_a_block(player)
    };
    let main = (9, MAIN_MUSIC_LOOP as i32);
    let river = (0, RIVER_LOOP as i32);
    let set = |list: u32, track: u32| (list as i32, track as i32);

    // Nothing until the neighbourhood (mode 3) has been shown.
    assert_eq!(mode(&mut player, 1), []);
    assert!(player.is_idle());
    // The neighbourhood's list, and the river.
    assert_eq!(
        mode(&mut player, 3),
        [river, set(NEIGHBOURHOOD, SET_MAIN_KILL_FIRST)]
    );
    assert_eq!(asked_in_a_block(&mut player), [river]);
    // Makin' Magic's neighbourhood: its list and river, and not the
    // other river.
    assert_eq!(
        mode(&mut player, 15),
        [
            (0, MT_RIVER_LOOP as i32),
            set(EXPANSION_LISTS[5], SET_MAIN_KILL_FIRST)
        ]
    );
    // Buy: its list, and no river.
    assert_eq!(mode(&mut player, 1), [set(BUY, SET_MAIN_MUSIC)]);
    assert!(player.is_idle());
    // Live after buy: the main music stops at once.
    play(&mut player, &format!("e{MAIN_MUSIC_LOOP}"));
    assert_eq!(asked_in_a_block(&mut player), [main]);
    assert_eq!(mode(&mut player, 0), []);
    assert!(player.is_idle());
    // The neighbourhood again, then live: the river stops and the main
    // music fades out (by its script).
    play(&mut player, &format!("e{MAIN_MUSIC_LOOP}"));
    assert_eq!(
        mode(&mut player, 3),
        [river, main, set(NEIGHBOURHOOD, SET_MAIN_KILL_FIRST)]
    );
    assert_eq!(
        mode(&mut player, 0),
        [main, set(MAIN_MUSIC_LOOP, FADE_AND_KILL_SLOW)]
    );
    // The credits play the newest expansion's list.
    assert_eq!(credits(EXPANSIONS), 103_101);
    assert_eq!(credits(0), NEIGHBOURHOOD);
    assert_eq!(credits(0x06), 1_589);
    assert_eq!(
        mode(&mut player, 6),
        [main, set(103_101, SET_MAIN_KILL_FIRST)]
    );
    // Makin' Magic's build, then its buy: the list changes once.
    assert_eq!(
        mode(&mut player, 17),
        [main, set(EXPANSION_LISTS[7], SET_MAIN_KILL_FIRST)]
    );
    assert_eq!(mode(&mut player, 18), [main]);
    // Options: nothing; the same mode again: nothing.
    assert_eq!(mode(&mut player, 7), [main]);
    assert_eq!(mode(&mut player, 7), [main]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn play_trk_hands_on_the_arguments_and_source_and_kill_trk_ends_the_whole_track() {
    // Track 10 asks about arg1's field arg2, then arg3's field `source`,
    // every block.
    let mut asker = vec![SET_LOOP];
    asker.extend([GETSRCDATAFIELD, 6, 1, 2, GETSRCDATAFIELD, 6, 3, 47, LOOP]);
    // Track 11: arg2 = 7, arg3 = 8, source = 5, then play_trk 10.
    let mut starter = [loadb(2, 7), loadb(3, 8), loadb(47, 5), loadb(5, 10)].concat();
    starter.extend([PLAY_TRK, 5, END]);
    // Track 12: kill_trk 10; track 13: kill_trk 13, then ask.
    let mut killer = loadb(5, 10).to_vec();
    killer.extend([KILL_TRK, 5, END]);
    let mut suicide = loadb(5, 13).to_vec();
    suicide.extend([KILL_TRK, 5, GETSRCDATAFIELD, 6, 1, 5, END]);
    let module = module_of_tracks(
        &[(10, &asker), (11, &starter), (12, &killer), (13, &suicide)],
        "",
    );
    let mut player = Player::new(vec![module], samples(), 1);
    // Started for object 9: the caller's own arguments go on. Track 11
    // runs in the first block, and track 10 from the next.
    play(&mut player, "e11");
    assert_eq!(asked_in_a_block(&mut player), []);
    assert_eq!(asked_in_a_block(&mut player), [(8, 5), (9, 7)]);
    // Another object's kill_trk ends it all the same: track 10 asks in
    // that block before track 12 runs, and not after.
    play_for(&mut player, 4, "e12");
    assert_eq!(asked_in_a_block(&mut player), [(8, 5), (9, 7)]);
    assert_eq!(asked_in_a_block(&mut player), []);
    assert!(player.is_idle());
    // A script that ends its own track ends there.
    play(&mut player, "e13");
    assert_eq!(asked_in_a_block(&mut player), []);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn play_trk_reads_source_through_the_target_but_the_arguments_from_its_own() {
    // Track 10 asks about arg1's field arg2, then arg3's field `source`,
    // every block. Track 14 holds `source` 6 and loops.
    let mut asker = vec![SET_LOOP];
    asker.extend([GETSRCDATAFIELD, 6, 1, 2, GETSRCDATAFIELD, 6, 3, 47, LOOP]);
    let mut holder = loadb(47, 6).to_vec();
    holder.extend([SET_LOOP, LOOP]);
    // Track 15: arg2 = 7, arg3 = 8, source = 5, all its own, then targets
    // track 14 and plays track 10.
    let mut starter = [
        loadb(2, 7),
        loadb(3, 8),
        loadb(47, 5),
        loadb(53, 14),
        loadb(5, 10),
    ]
    .concat();
    starter.extend([PLAY_TRK, 5, END]);
    let module = module_of_tracks(&[(10, &asker), (14, &holder), (15, &starter)], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e14");
    player.render(BLOCK, &NoSources);
    play(&mut player, "e15");
    assert_eq!(asked_in_a_block(&mut player), []);
    assert_eq!(asked_in_a_block(&mut player), [(8, 6), (9, 7)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn mp3_patches_play_at_the_output_rate() {
    use sims_audio::player::resample;
    let pcm = Pcm {
        rate: 2 * RATE,
        channels: 2,
        samples: vec![0, 10, 100, 110, 200, 210, 300, 310],
    };
    let half = resample(&pcm, RATE).unwrap();
    assert_eq!(half.rate, RATE);
    assert_eq!(half.samples, [0, 10, 200, 210]);
    let up = resample(
        &Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![0, 100],
        },
        2 * RATE,
    )
    .unwrap();
    assert_eq!(up.samples, [0, 50, 100, 100]);
    assert!(resample(&Pcm::default(), RATE).unwrap().samples.is_empty());
    // A result past half an hour is refused, not allocated.
    let slow = |frames: usize, rate| Pcm {
        rate,
        channels: 1,
        samples: vec![0; frames],
    };
    assert_eq!(resample(&slow(1, 1), u32::MAX), None);
    assert_eq!(resample(&slow(2000, 1), RATE), None);
    assert!(resample(&slow(1799, 1), RATE).is_some());
    // A patch whose file is an MP3 is resampled when loaded.
    let mut hot = module_with(&[&GENERIC], 1, 10, "");
    hot.hot.patches.get_mut(&1).unwrap().path = "one.mp3".into();
    let mut files = samples();
    files.insert("one.mp3".into(), pcm);
    let mut player = Player::new(vec![hot], files, 1);
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [5, 205, 0]);
}

#[test]
fn a_mp3_too_long_once_resampled_is_a_fault() {
    let mut module = module_with(&[&GENERIC], 1, 10, "");
    module.hot.patches.get_mut(&1).unwrap().path = "one.mp3".into();
    let mut files = samples();
    files.insert(
        "one.mp3".into(),
        Pcm {
            rate: 1,
            channels: 1,
            samples: vec![1; 2000],
        },
    );
    let mut player = Player::new(vec![module], files, 1);
    play(&mut player, "e0");
    player.render(BLOCK, &NoSources);
    assert!(player.faults[0].contains("too long"), "{:?}", player.faults);
    assert!(player.is_idle());
}

#[test]
fn an_object_that_goes_fades_a_track_still_running_after_end() {
    // e0 plays a note and reaches `end`, then runs on into a wait loop
    // while fading out (0x57e3ee → 0x57dca3). Object 2 playing it
    // refreshes the running track and cancels the fade (0x57d9b2) with
    // one voice still playing; object 1 going leaves object 2 playing;
    // object 2 going fades it out over 11 blocks and stops it.
    let routine: &[u8] = &[2, 5, END];
    let mut hold_pc = vec![SET_LOOP];
    hold_pc.extend(loadl(5, 1000));
    hold_pc.extend([WAIT, 5, LOOP]);
    let module = module_with(&[routine, &hold_pc], 1, 10, "");
    let mut player = Player::new(vec![module], loud(30 * BLOCK), 1);
    play_for(&mut player, 1, "e0");
    player.render(BLOCK, &NoSources);
    play_for(&mut player, 2, "e0");
    player.render(BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [10_000]);
    // Object 2 still has the track, so it keeps playing at full volume.
    player.handle(&SoundEvent::StopObject { object: 1 }, &NoSources);
    player.render(2 * BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [10_000]);
    // The last object going fades it out at speed 100 and ends it.
    player.handle(&SoundEvent::StopObject { object: 2 }, &NoSources);
    player.render(2 * BLOCK, &NoSources);
    let fading = player.render(1, &NoSources)[0];
    assert!(fading > 0 && fading < 10_000, "{fading}");
    player.render(10 * BLOCK, &NoSources);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn sequenced_patches_on_a_track_with_no_thread_chain_to_each_other() {
    let module = module_with(&[&GENERIC], 1, 10, "seq=53,10,0,6\n");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "seq");
    play(&mut player, "seq");
    assert_eq!(
        player.render(7, &NoSources),
        [2000, 4000, -2000, 2000, 4000, -2000, 0]
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn event_numbers_past_32_bits_are_faults() {
    let events = "seq=53,10,0,4294967302\nplay=1,4294967306\nok=53,10,0,-1\n";
    let module = module_with(&[&GENERIC], 1, 10, events);
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "seq");
    play(&mut player, "play");
    assert!(player.is_idle());
    assert_eq!(player.faults.len(), 2, "{:?}", player.faults);
    assert!(
        player
            .faults
            .iter()
            .all(|f| f.contains("does not fit 32 bits"))
    );
    // -1 fits (as 0xffffffff): no such hit list.
    play(&mut player, "ok");
    assert!(player.faults[2].contains("no hit list 4294967295"));
}

const IFLE: u8 = 67;

#[test]
fn a_fresh_start_leaves_positive_flags_whatever_its_third_argument() {
    // Branch to the silent routine 0x101 unless the flags are positive; the
    // event's third argument is -1. A fresh start writes isplaying (46)
    // last (0x57dbc5), so the flags are positive and the note plays.
    let mut branches = vec![IFLE];
    branches.extend(long(0x101));
    branches.extend([2, 5, WAIT_SAMP, END]);
    let silent: &[u8] = &[END];
    let module = module_with(&[&branches, silent], 1, 10, "neg=1,10,0,0,-1\n");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "neg");
    assert_eq!(player.render(5, &NoSources), [1, 2, 3, 4, 5]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_television_turned_on_during_a_pause_waits_for_the_unpause() {
    // Kind 30 starts the track but resumes it only when the game is not
    // paused (0x573d71); the start held it (0x57dc2e).
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let module = module_with(&[generic], 1, 10, "on=30,10,3\n");
    let mut player = Player::new(vec![module], samples(), 1);
    player.handle(&SoundEvent::Pause, &NoSources);
    play(&mut player, "on");
    assert!(player.render(2 * BLOCK, &NoSources).iter().all(|&s| s == 0));
    player.handle(&SoundEvent::Unpause, &NoSources);
    let out = player.render(2 * BLOCK, &NoSources);
    let first = out.iter().position(|&s| s != 0).expect("the note plays");
    assert_eq!(&out[first..first + 5], [1, 2, 3, 4, 5]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

/// A snapshot with these objects on the lot, each on the screen at
/// `x` across an 800 by 600 screen, halfway down, at the near zoom.
fn on_lot(objects: &[(i32, i32)]) -> Snapshot {
    let mut snapshot = Snapshot::default();
    for &(object, x) in objects {
        snapshot.positions.insert(object, [40, 56, 1]);
        snapshot.screen.insert(
            object,
            Projection {
                at: [x, 300],
                ..MIDDLE
            },
        );
    }
    snapshot
}

fn play_on(player: &mut Player<BTreeMap<String, Pcm>>, object: u32, lot: &Snapshot) {
    player.handle(
        &SoundEvent::Play {
            object,
            name: "e0".into(),
        },
        lot,
    );
}

#[test]
fn a_sound_is_panned_toward_where_its_object_is_on_the_screen() {
    // At the right edge the pan is 542 and the volume 750 (0x5730b6);
    // in the middle, 512 and full. An object not on the lot leaves the
    // sum's start, a heap address for the volume, which outweighs the
    // pan's 0 to the centre.
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    for (lot, right) in [
        (on_lot(&[(9, 800)]), true),
        (on_lot(&[(9, 400)]), false),
        (on_lot(&[]), false),
    ] {
        let mut player = Player::new(vec![module(&[generic], 1)], samples(), 1);
        play_on(&mut player, 9, &lot);
        let out = player.render_stereo(100, &lot);
        let [left, right_side] = [out[198], out[199]];
        assert!(left != 0, "{out:?}");
        assert_eq!(right_side > left, right, "{left} {right_side}");
        assert!(right || right_side == left, "{left} {right_side}");
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

/// A lot that also notes each field asked, as (object, field).
struct Probe {
    lot: Snapshot,
    asked: std::cell::RefCell<Vec<(i32, i32)>>,
}

impl Sources for Probe {
    fn field(&self, source: i32, field: i32) -> Option<i32> {
        self.asked.borrow_mut().push((source, field));
        Some(0)
    }

    fn position(&self, object: i32) -> Option<[i32; 3]> {
        self.lot.position(object)
    }

    fn projection(&self, object: i32) -> Option<Projection> {
        self.lot.projection(object)
    }
}

#[test]
fn a_track_s_objects_go_first_recorded_then_newest_first() {
    // Object 5 is in the middle of the screen and 4 is not on the lot,
    // so 4 takes the values of the object before it (0x57473b), or the
    // sum's stale start (a heap address for the volume) when it is
    // first. The script asks for the field its extvol names.
    let generic: &[u8] = &[72, 6, 1, 19, END]; // getsrcdatafield v2, arg1, extvol
    let stale = sims_audio::placement::STALE_VOL;
    for (order, extvol) in [([5, 4], 1024), ([4, 5], stale)] {
        let probe = Probe {
            lot: on_lot(&[(5, 400)]),
            asked: Default::default(),
        };
        let mut player = Player::new(vec![module(&[generic], 1)], samples(), 1);
        for object in order {
            player.handle(
                &SoundEvent::Play {
                    object,
                    name: "e0".into(),
                },
                &probe,
            );
        }
        player.render(BLOCK, &probe);
        // The second start's objects go first recorded, then the new
        // one (0x5761c2).
        let second = i32::from(order[1] as u8);
        assert!(
            probe.asked.borrow().contains(&(second, extvol)),
            "{:?}",
            probe.asked.borrow()
        );
    }
    // Recorded 5 (at the right edge: 750 at 542), 6 (in the middle) and
    // 4 (not on the lot), they go 5, 4, 6, and 4 takes 5's values: the
    // pan is 512 + 2 * 30 * 750 / 2524. In the order recorded, 4 would
    // take 6's, and the pan be 520.
    let generic: &[u8] = &[72, 6, 1, 20, END]; // getsrcdatafield v2, arg1, pan
    let probe = Probe {
        lot: on_lot(&[(5, 800), (6, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(vec![module(&[generic], 1)], samples(), 1);
    for object in [5, 6, 4] {
        player.handle(
            &SoundEvent::Play {
                object,
                name: "e0".into(),
            },
            &probe,
        );
    }
    player.render(BLOCK, &probe);
    assert!(
        probe.asked.borrow().contains(&(4, 529)),
        "{:?}",
        probe.asked.borrow()
    );
}

#[test]
fn the_placement_s_pan_sets_the_comparison_flags_when_the_event_comes() {
    // The script sets Spl to 10001 (whose volume never falls below 800)
    // and waits a second; the pause holds it. Started again, it is
    // placed with that Spl when the event comes (0x573795 to 0x5737a4):
    // far off the left of the screen, the pan is 0 and the flags with
    // it, and the script's `ifle` goes to the silent routine 0x101; in
    // the middle, the pan is 512 and the script asks about its object. A
    // kind 53 after it writes the patch (`0x57e0cc`), whose `0x57f6e3`..`0x57f6fb`
    // copies the patch's `Is3d` flag into register `0x1a` via `vtable +0x84`,
    // overwriting the comparison flags with `is_3d` (0 for 2D, 1 for 3D).
    let mut branches = vec![LOADL, 57];
    branches.extend(long(10_001));
    branches.extend([LOADL, 5]);
    branches.extend(long(1000));
    branches.extend([WAIT, 5, IFLE]);
    branches.extend(long(0x101));
    branches.extend([72, 6, 1, 2, WAIT_SAMP, END]); // getsrcdatafield v2, arg1, arg2
    let silent: &[u8] = &[WAIT_SAMP, END];
    let (hit, addresses) = assemble(&[&branches, silent]);
    let hot = format!(
        "[EventMappingEquate]\n\
         kSndobPlay=1\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\",0,0,0,0,0\n\
         2=0,two,\"two.xa\",0,0,0,0,0\n\
         3=0,two3d,\"two.xa\",0,0,0,1,0\n\
         [GlobalHitList]\n\
         6=2\n\
         7=3\n\
         [Track]\n\
         10=0,t0,1024,2,0,0,0,0x100,1\n\
         11=0,t1,1024,2,0,0,0,0x101,1\n\
         [EventMapping]\n\
         e0=kSndobPlay,10\n\
         e1=kSndobPlay,11\n\
         seq2d=53,10,0,6\n\
         seq3d=53,10,0,7\n",
        addresses[0], addresses[1]
    );
    for (x, seq_event, asks) in [
        (400, None, true),
        (-7000, None, false),
        (400, Some("seq2d"), false),
        (-7000, Some("seq3d"), true),
    ] {
        let probe = Probe {
            lot: on_lot(&[(9, x)]),
            asked: Default::default(),
        };
        let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
        let mut player = Player::new(vec![module], samples(), 1);
        let event = |name: &str| SoundEvent::Play {
            object: 9,
            name: name.into(),
        };
        player.handle(&event("e0"), &probe);
        player.render(BLOCK, &probe);
        player.handle(&SoundEvent::Pause, &probe);
        player.handle(&event("e0"), &probe);
        if let Some(seq) = seq_event {
            player.handle(&event(seq), &probe);
        }
        player.handle(&SoundEvent::Unpause, &probe);
        player.render(2 * BLOCK, &probe);
        assert_eq!(!probe.asked.borrow().is_empty(), asks, "{x} {seq_event:?}");
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn a_camera_move_places_every_track_again() {
    // The script waits 10 ms, then asks about its object with the field
    // its extvol and then its pan name. Started for object 9 in the
    // middle of the screen, the track is at full volume and the centre.
    // When the camera moves and the object is at the right edge, kind 33
    // places it again (0x57447f): 750 at 542 (0x5730b6). A camera that
    // has not moved leaves it where the start put it.
    let mut routine = vec![LOADL, 5];
    routine.extend(long(10));
    routine.extend([WAIT, 5]);
    routine.extend([72, 6, 1, 19, 72, 6, 1, 20, END]);
    for (moved, extvol, pan) in [(true, 750, 542), (false, 1024, 512)] {
        let start = on_lot(&[(9, 400)]);
        let probe = Probe {
            lot: on_lot(&[(9, 800)]),
            asked: Default::default(),
        };
        let mut player = Player::new(vec![module(&[&routine], 1)], samples(), 1);
        play_on(&mut player, 9, &start);
        player.render(BLOCK, &probe);
        if moved {
            player.camera_moved(0, u32::MAX, &probe);
        }
        player.render(8 * BLOCK, &probe);
        assert_eq!(*probe.asked.borrow(), [(9, extvol), (9, pan)], "{moved}");
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn a_camera_move_leaves_a_track_that_has_ended() {
    // The track's thread has ended with no note playing, so the camera
    // move has nothing to write to, asks nothing and faults nothing; its
    // next start is placed afresh.
    let generic: &[u8] = &[72, 6, 1, 20, END]; // getsrcdatafield v2, arg1, pan
    let probe = Probe {
        lot: on_lot(&[(9, 800)]),
        asked: Default::default(),
    };
    let mut player = Player::new(vec![module(&[generic], 1)], samples(), 1);
    play_on(&mut player, 9, &on_lot(&[(9, 400)]));
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(9, 512)]);
    player.camera_moved(0, u32::MAX, &probe);
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(9, 512)]);
    play_on(&mut player, 9, &probe.lot);
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(9, 512), (9, 542)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_camera_move_places_a_note_fading_after_end() {
    // `note_on; end` (followed by an explicit wait): the track stays
    // running while fading out at speed 100 (0x57e3ee → 0x57dca3), and
    // kind 33's write reaches its fading note: started in the middle of
    // the screen, it is level both sides; once the camera has moved and
    // the object is at the right edge, it is louder on the right.
    let routine: &[u8] = &[2, 5, END];
    let mut hold_pc = loadl(5, 1000);
    hold_pc.extend([WAIT, 5]);
    let long = long_sample("one.xa");
    let start = on_lot(&[(9, 400)]);
    let right = on_lot(&[(9, 800)]);
    for moved in [true, false] {
        let mut player = Player::new(vec![module(&[routine, &hold_pc], 1)], long.clone(), 1);
        play_on(&mut player, 9, &start);
        let out = player.render_stereo(2 * BLOCK, &start);
        let last = out.len() - 2;
        assert_eq!(out[last], out[last + 1], "{out:?}");
        assert_ne!(out[last], 0);
        if moved {
            player.camera_moved(0, u32::MAX, &right);
        }
        let out = player.render_stereo(2 * BLOCK, &right);
        let last = out.len() - 2;
        assert_eq!(out[last + 1] > out[last], moved, "{:?}", &out[last..]);
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

/// The test samples with `name` replaced by 20 blocks of 10,000.
fn long_sample(name: &str) -> BTreeMap<String, Pcm> {
    let mut map = samples();
    map.insert(
        name.to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![10_000; 20 * BLOCK],
        },
    );
    map
}

#[test]
fn a_camera_move_does_not_follow_a_target_a_later_start_reset() {
    // Track 10's first start sets its `target` to track 11; global 110
    // marks it, and its second start resets `target` back to itself
    // (0x57d9c3) and runs routine 2 (or refreshes in place). Track 11
    // waits 10 ms for object 8, which does not move, then asks with its
    // extvol and pan. The camera move places track 10 itself, so track
    // 11 keeps the full volume and the centre its own start gave it.
    let mark = |interrupt: bool, then: &[u8]| {
        let mut first = vec![TEST, 110, IFNE];
        first.extend(long(0x102));
        first.extend(loadl(110, 1));
        if interrupt {
            first.extend(loadb(55, 1));
        }
        first.extend(loadl(53, 11));
        first.extend(then);
        first
    };
    let wait: &[u8] = &[SET_LOOP, LOOP];
    let note_and_end: &[u8] = &[NOTE_ON_V1[0], NOTE_ON_V1[1], END];
    let cases: [(Vec<u8>, &[u8]); 3] = [
        (mark(true, wait), wait),
        (mark(false, &[END]), &[END]),
        (mark(true, wait), note_and_end),
    ];
    let mut asks = loadl(5, 10);
    asks.extend([WAIT, 5]);
    asks.extend([72, 6, 1, 19, 72, 6, 1, 20, END]);
    let start = on_lot(&[(8, 400), (9, 400)]);
    for (case, (first, second)) in cases.iter().enumerate() {
        let probe = Probe {
            lot: on_lot(&[(8, 400), (9, 800)]),
            asked: Default::default(),
        };
        let mut player = Player::new(
            vec![module(&[first, &asks, second, wait], 1)],
            long_sample("one.xa"),
            1,
        );
        let play = |player: &mut Player<_>, object: u32, name: &str| {
            player.handle(
                &SoundEvent::Play {
                    object,
                    name: name.into(),
                },
                &start,
            );
        };
        play(&mut player, 8, "e1");
        play(&mut player, 9, "e0");
        player.render(BLOCK, &start);
        play(&mut player, 9, "e0");
        player.render(BLOCK, &start);
        player.camera_moved(0, u32::MAX, &probe);
        player.render(8 * BLOCK, &probe);
        assert_eq!(*probe.asked.borrow(), [(8, 1024), (8, 512)], "case {case}");
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn a_camera_move_reaches_a_target_fading_after_end() {
    // Track 11 (object 8, which stays in the middle) plays a long note,
    // waits and reaches `end` (running on into `set_loop; loop` while
    // fading out at speed 100, 0x57dca3). Track 10 (object 9) starts
    // while it waits, sets its `target` to track 11 and loops. While
    // track 11 is still running and fading, the camera move writes
    // through track 10's `target` to track 11: with object 9 at the
    // right edge, the note turns right.
    let mut fading = NOTE_ON_V1.to_vec();
    fading.extend(loadl(5, 10));
    fading.extend([WAIT, 5, END]);
    let mut targets = loadl(53, 11);
    targets.extend(loadl(5, 1000));
    targets.extend([WAIT, 5]);
    let mut hold_pc = loadl(5, 1000);
    hold_pc.extend([WAIT, 5]);
    let start = on_lot(&[(8, 400), (9, 400)]);
    let right = on_lot(&[(8, 400), (9, 800)]);
    for moved in [true, false] {
        let mut player = Player::new(
            vec![module(&[&targets, &fading, &hold_pc], 1)],
            long_sample("one.xa"),
            1,
        );
        let play = |player: &mut Player<_>, object: u32, name: &str| {
            player.handle(
                &SoundEvent::Play {
                    object,
                    name: name.into(),
                },
                &start,
            );
        };
        play(&mut player, 8, "e1");
        player.render(BLOCK, &start);
        play(&mut player, 9, "e0");
        player.render(7 * BLOCK, &start);
        if moved {
            player.camera_moved(0, u32::MAX, &right);
        }
        let out = player.render_stereo(2 * BLOCK, &right);
        let last = out.len() - 2;
        assert_ne!(out[last], 0);
        assert_eq!(out[last + 1] > out[last], moved, "{:?}", &out[last..]);
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn a_camera_move_goes_no_further_than_a_thread_whose_track_started_again() {
    // Track 12 (object 7, in the middle) waits 100 ms, then asks with its
    // extvol and pan. Track 11 (object 8) plays a long note, waits, sets
    // its `target` to track 12 and loops; global 110 marks it, so a
    // second start resets `target` back to itself (0x57d9c3). Track 10
    // (object 9) starts while track 11 waits, sets its `target` to it
    // and loops. With object 9 at the right edge, the camera move goes
    // from track 10 through track 11 to track 12, which turns right.
    // Once track 11 has been played again, its `target` points back at
    // itself and track 12 keeps the centre.
    let mut middle = vec![TEST, 110, IFNE];
    middle.extend(long(0x103));
    middle.extend(loadl(110, 1));
    middle.extend(NOTE_ON_V1);
    middle.extend(loadl(5, 10));
    middle.extend([WAIT, 5]);
    middle.extend(loadl(53, 12));
    middle.extend([SET_LOOP, LOOP]);
    let mut targets = loadl(53, 11);
    targets.extend([SET_LOOP, LOOP]);
    let mut asks = loadl(5, 100);
    asks.extend([WAIT, 5]);
    asks.extend([72, 6, 1, 19, 72, 6, 1, 20, END]);
    let wait: &[u8] = &[SET_LOOP, LOOP];
    let start = on_lot(&[(7, 400), (8, 400), (9, 400)]);
    for restarted in [true, false] {
        let probe = Probe {
            lot: on_lot(&[(7, 400), (8, 400), (9, 800)]),
            asked: Default::default(),
        };
        let mut player = Player::new(
            vec![module(&[&targets, &middle, &asks, wait], 1)],
            long_sample("one.xa"),
            1,
        );
        let play = |player: &mut Player<_>, object: u32, name: &str| {
            player.handle(
                &SoundEvent::Play {
                    object,
                    name: name.into(),
                },
                &start,
            );
        };
        play(&mut player, 7, "e2");
        player.render(BLOCK, &start);
        play(&mut player, 8, "e1");
        player.render(BLOCK, &start);
        play(&mut player, 9, "e0");
        player.render(5 * BLOCK, &start);
        if restarted {
            play(&mut player, 8, "e1");
        }
        player.render(BLOCK, &start);
        player.camera_moved(0, u32::MAX, &probe);
        player.render(40 * BLOCK, &probe);
        let asked = probe.asked.borrow().clone();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert_eq!(asked[1] == (7, 512), restarted, "{asked:?}");
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn a_stop_with_a_stopped_target_kills_self_next_block_and_camera_leaves_self() {
    // In 1.91 (0x57e023 → 0x57de57), `target` holds the registered track
    // object even after that track stops. When track 10 targets stopped
    // track 11:
    // 1) A camera move writes to stopped track 11 (not falling back to
    //    track 10), so track 10 keeps its centered placement.
    // 2) Stopping object 9 calls 0x57dca3, which sends `fade_on = 1` to
    //    stopped track 11 (where 0x57e059..0x57e064 clears it to 0) and
    //    sets `kill_after_fade` locally on track 10 (+0x1cf). Because
    //    track 10's own `fade_on` is 0, track 10 kills itself on the
    //    very next update (0x57d86e → 0x57dd3e).
    let mut ended = loadl(5, 10);
    ended.extend([WAIT, 5, END]);
    let mut targets = NOTE_ON_V1.to_vec();
    targets.extend(loadl(53, 11));
    targets.extend(loadl(5, 1000));
    targets.extend([WAIT, 5, END]);
    let start = on_lot(&[(8, 400), (9, 400)]);
    let right = on_lot(&[(8, 400), (9, 800)]);
    let mut player = Player::new(
        vec![module(&[&targets, &ended], 1)],
        long_sample("one.xa"),
        1,
    );
    let play = |player: &mut Player<_>, object: u32, name: &str| {
        player.handle(
            &SoundEvent::Play {
                object,
                name: name.into(),
            },
            &start,
        );
    };
    play(&mut player, 8, "e1");
    player.render(BLOCK, &start);
    play(&mut player, 9, "e0");
    // Wait until track 11 has ended (10 ms ≈ 4 blocks).
    player.render(5 * BLOCK, &start);
    assert_eq!(player.render_stereo(1, &start), [10_000, 10_000]);

    // Camera move routes through target to stopped track 11, leaving
    // track 10's own note centered at [10_000, 10_000].
    player.camera_moved(0, u32::MAX, &right);
    assert_eq!(player.render_stereo(1, &right), [10_000, 10_000]);

    // StopObject sends fade_on=1 to stopped track 11 and sets local
    // kill_after_fade on track 10; next block kills track 10 at once.
    player.handle(&SoundEvent::StopObject { object: 9 }, &right);
    let rest_of_block = BLOCK - (player.frame() % BLOCK as u64) as usize;
    player.render(rest_of_block, &right);
    assert!(player.render(BLOCK, &right).iter().all(|&s| s == 0));
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_camera_move_places_a_sequenced_note_with_no_thread() {
    // Track 10's routine ends at once, so kind 53 plays its long patch
    // on the idle track object (thread 0). Started in the middle of the
    // screen it is level both sides; once the camera has moved and the
    // object is at the right edge, it is louder on the right.
    let start = on_lot(&[(9, 400)]);
    let right = on_lot(&[(9, 800)]);
    for moved in [true, false] {
        let module = module_with(&[&[END]], 1, 10, "seq=53,10,0,6\n");
        let mut player = Player::new(vec![module], long_sample("two.xa"), 1);
        play_on(&mut player, 9, &start);
        player.render(BLOCK, &start);
        player.handle(
            &SoundEvent::Play {
                object: 9,
                name: "seq".into(),
            },
            &start,
        );
        let out = player.render_stereo(2 * BLOCK, &start);
        let last = out.len() - 2;
        assert_eq!(out[last], out[last + 1], "{out:?}");
        assert_ne!(out[last], 0);
        if moved {
            player.camera_moved(0, u32::MAX, &right);
        }
        let out = player.render_stereo(2 * BLOCK, &right);
        let last = out.len() - 2;
        assert_eq!(out[last + 1] > out[last], moved, "{:?}", &out[last..]);
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn a_camera_move_follows_a_held_track_let_go_by_a_start() {
    // Track 10's first start marks global 110 and loops; its second plays
    // a long note, sets its `target` to track 11 and ends. With the sound
    // paused, a third start lets the held loop go (0x57d9f1), and every
    // start sets the track object's `target` back to itself (0x57d9c3):
    // the camera move then places the loop itself, and track 11 (object
    // 8, which does not move) keeps its own levels (Sol's review).
    let mut first = vec![TEST, 110, IFNE];
    first.extend(long(0x102));
    first.extend(loadl(110, 1));
    first.extend([SET_LOOP, LOOP]);
    let mut second = NOTE_ON_V1.to_vec();
    second.extend(loadl(53, 11));
    second.push(END);
    let mut asks = loadl(5, 10);
    asks.extend([WAIT, 5]);
    asks.extend([72, 6, 1, 19, 72, 6, 1, 20, END]);
    let start = on_lot(&[(8, 400), (9, 400)]);
    let probe = Probe {
        lot: on_lot(&[(8, 400), (9, 800)]),
        asked: Default::default(),
    };
    let mut player = Player::new(
        vec![module(&[&first, &asks, &second], 1)],
        long_sample("one.xa"),
        1,
    );
    let play = |player: &mut Player<_>, object: u32, name: &str| {
        player.handle(
            &SoundEvent::Play {
                object,
                name: name.into(),
            },
            &start,
        );
    };
    play(&mut player, 8, "e1");
    play(&mut player, 9, "e0");
    player.render(BLOCK, &start);
    play(&mut player, 9, "e0");
    player.render(BLOCK, &start);
    player.handle(&SoundEvent::Pause, &start);
    play(&mut player, 9, "e0");
    player.camera_moved(0, u32::MAX, &probe);
    player.handle(&SoundEvent::Unpause, &probe);
    player.render(8 * BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(8, 1024), (8, 512)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_camera_move_reads_spl_where_it_writes() {
    // Track 11 (object 8) sets Spl to 10001, whose volume never falls
    // below 800, plays a long note, waits and reaches `end` (continuing
    // into `set_loop; loop` while fading out, 0x57dca3). Track 10
    // (object 9) sets its `target` to track 11 and loops. The camera
    // move puts object 9 far off the screen; track 10's write goes
    // through to track 11, and so does its read of Spl: the fading note
    // is quiet but heard. Read from track 10, whose Spl is the usual 20,
    // the note would fall silent.
    let mut fading = vec![LOADL, 57];
    fading.extend(long(10_001));
    fading.extend(NOTE_ON_V1);
    fading.extend(loadl(5, 10));
    fading.extend([WAIT, 5, END]);
    let mut targets = loadl(53, 11);
    targets.extend(loadl(5, 1000));
    targets.extend([WAIT, 5]);
    let mut hold_pc = loadl(5, 1000);
    hold_pc.extend([WAIT, 5]);
    let start = on_lot(&[(8, 400), (9, 400)]);
    let off = on_lot(&[(8, 400), (9, -7000)]);
    let mut player = Player::new(
        vec![module(&[&targets, &fading, &hold_pc], 1)],
        long_sample("one.xa"),
        1,
    );
    let play = |player: &mut Player<_>, object: u32, name: &str| {
        player.handle(
            &SoundEvent::Play {
                object,
                name: name.into(),
            },
            &start,
        );
    };
    play(&mut player, 8, "e1");
    player.render(BLOCK, &start);
    play(&mut player, 9, "e0");
    player.render(7 * BLOCK, &start);
    player.camera_moved(0, u32::MAX, &off);
    let out = player.render_stereo(2 * BLOCK, &off);
    let last = out.len() - 2;
    assert_ne!(out[last..], [0, 0], "{:?}", &out[last..]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_forwarded_camera_move_leaves_the_forwarding_thread_s_levels() {
    // Track 10 (object 9) plays a long note in the middle, sets its
    // `target` to track 11 (object 8), and loops. When the camera moves
    // object 9 to the right edge, track 10's placement goes through its
    // `target` to track 11, so track 10's own note keeps its centered
    // levels (left == right).
    let mut forwards = NOTE_ON_V1.to_vec();
    forwards.extend(loadl(53, 11));
    forwards.extend([SET_LOOP, LOOP]);
    let wait: &[u8] = &[SET_LOOP, LOOP];
    let start = on_lot(&[(8, 400), (9, 400)]);
    let moved = on_lot(&[(8, 400), (9, 800)]);
    let mut player = Player::new(
        vec![module(&[&forwards, wait], 1)],
        long_sample("one.xa"),
        1,
    );
    let play = |player: &mut Player<_>, object: u32, name: &str| {
        player.handle(
            &SoundEvent::Play {
                object,
                name: name.into(),
            },
            &start,
        );
    };
    play(&mut player, 8, "e1");
    play(&mut player, 9, "e0");
    player.render(BLOCK, &start);
    let before = player.render_stereo(1, &start);
    assert!(before[0] > 0 && before[0] == before[1], "{before:?}");
    player.camera_moved(0, u32::MAX, &moved);
    let after = player.render_stereo(1, &moved);
    assert_eq!(after, before);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_track_that_ends_another_and_restarts_itself_skips_no_track() {
    // Track 10 waits; track 11 ends it and starts itself anew (interrupt
    // 1); track 12 plays patch 1. All run in the first block: removing
    // track 10 and restarting 11 must not make the block skip 12.
    let waiting: &[u8] = &[SET_LOOP, LOOP];
    let restarter: &[u8] = &[
        LOADB_V1, 55, 1, LOADB_V1, 5, 10, KILL_TRK, 5, LOADB_V1, 5, 11, PLAY_TRK, 5, END,
    ];
    let module = module(&[waiting, restarter, &GENERIC], 1);
    let mut player = Player::new(vec![module], samples(), 1);
    for name in ["e0", "e1", "e2"] {
        play(&mut player, name);
    }
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_restart_keeps_the_track_its_targets_and_its_locals() {
    // Track 11 counts its starts from the top in register 4 (outside the
    // default banks) and waits. Track 10 points `target` at it and asks
    // about its register 4 every block. Played again with interrupt 1,
    // track 11 starts anew as the same track: the count goes on, and
    // track 10 still reaches it. Let go, track 11 now runs after track
    // 10, so track 10 sees the count kept first.
    let counter: &[u8] = &[LOADB_V1, 55, 1, ADD, 4, 55, SET_LOOP, LOOP];
    let mut watcher = loadl(53, 11);
    watcher.extend([SET_LOOP, GETSRCDATAFIELD, 6, 4, 4, LOOP]);
    let module = module(&[&watcher, counter], 1);
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e1");
    play(&mut player, "e0");
    assert_eq!(asked_in_a_block(&mut player), [(1, 1)]);
    play(&mut player, "e1");
    assert_eq!(asked_in_a_block(&mut player), [(1, 1)]);
    assert_eq!(asked_in_a_block(&mut player), [(2, 2)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn play_trk_reads_source_after_the_start() {
    // Track 10 sets its source to 9, points `target` at track 11 (source
    // 7) and plays itself: the start points its `target` back at itself
    // first, so the source it hands on is its own 9.
    let other: &[u8] = &[LOADB_V1, 47, 7, SET_LOOP, LOOP];
    // (With `target` at track 11, v1 is written to and read from there.)
    let mut caller = vec![LOADB_V1, 47, 9];
    caller.extend(loadl(53, 11));
    caller.extend([LOADB_V1, 5, 10, PLAY_TRK, 5]);
    caller.extend([GETSRCDATAFIELD, 6, 47, 47, SET_LOOP, LOOP]);
    let module = module(&[&caller, other], 1);
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e1");
    play(&mut player, "e0");
    assert_eq!(asked_in_a_block(&mut player), [(9, 9)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_music_mode_lets_go_of_a_music_track_it_starts_again() {
    use sims_audio::player::music::*;
    // The buy and build music track waits a second, then asks about its
    // first argument.
    let mut waits = vec![LOADL, 5];
    waits.extend(long(1000));
    waits.extend([WAIT, 5]);
    waits.extend(asks(SET_MAIN_MUSIC, false));
    let tracks: Vec<(u32, Vec<u8>)> = vec![
        (SET_MAIN_MUSIC, waits),
        (SET_MAIN_KILL_FIRST, asks(SET_MAIN_KILL_FIRST, false)),
        (RIVER_LOOP, asks(RIVER_LOOP, false)),
    ];
    let tracks: Vec<(u32, &[u8])> = tracks.iter().map(|(t, r)| (*t, &r[..])).collect();
    let events: String = (0..=18).map(|m| format!("m{m}=36,{m}\n")).collect();
    let mut player = Player::new(vec![module_of_tracks(&tracks, &events)], samples(), 1);
    play(&mut player, "m3");
    asked_in_a_block(&mut player);
    play(&mut player, "m1");
    assert_eq!(asked_in_a_block(&mut player), []);
    // Build starts it again while it waits: it is let go, with the
    // build list.
    play(&mut player, "m2");
    assert_eq!(
        asked_in_a_block(&mut player),
        [(BUILD as i32, SET_MAIN_MUSIC as i32)]
    );
}

/// Renders a block; returns the fields the scripts asked about in it, in
/// the order asked.
fn fields_asked_in_order(player: &mut Player<BTreeMap<String, Pcm>>) -> Vec<i32> {
    let asked = Asked::default();
    player.render(BLOCK, &asked);
    asked.0.into_inner().into_iter().map(|(_, f)| f).collect()
}

#[test]
fn a_track_played_again_runs_after_the_others_from_the_next_block() {
    // Tracks 10 and 11 ask every block, in the order they started. Track
    // 10 played again is let go, which puts it last.
    let module = module_of_tracks(&[(10, &asks(10, true)), (11, &asks(11, true))], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e10");
    play(&mut player, "e11");
    assert_eq!(fields_asked_in_order(&mut player), [10, 11]);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [11, 10]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_restart_runs_the_registered_track_object_from_its_own_entry() {
    // Track 10 asks every block, with interrupt 1. Another module, listed
    // first, also has a track 10, elsewhere in its program, and the
    // events that play it; the asking module's line registers last, so
    // its track object is the one (0x57ce04). Both the start and the
    // restart run it, from its own entry.
    let mut first = vec![LOADB_V1, 55, 1];
    first.extend(asks(7, true));
    let mut padding = vec![NOP; 400];
    padding.push(END);
    let second = module_of_tracks(&[(99, &padding), (10, &[END])], "again=kSndobPlay,10\n");
    let modules = vec![second, module_of_tracks(&[(10, &first)], "")];
    let mut player = Player::new(modules, samples(), 1);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    play(&mut player, "again");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_track_ended_and_started_again_before_its_turn_runs_in_it() {
    // Track 10 ends track 11 and starts it again; track 11, after it in
    // the block, still has its turn there (the original's copy of its
    // list holds the same track object), and plays patch 1. (Track 11
    // takes no levels from its arguments, so it plays at full volume.)
    let killer: &[u8] = &[LOADB_V1, 5, 11, KILL_TRK, 5, PLAY_TRK, 5, END];
    let module = module_of_kinds(&[(10, 2, killer), (11, 0, &GENERIC)], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e10");
    play(&mut player, "e11");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_fade_that_ends_a_track_beyond_the_budget_ends_it_at_once() {
    // Track 10 asks once, from the top, then waits. Twenty threads that
    // spend 10,000 instructions once and end come after it: the second
    // ten run first in the next block (they were not reached) and use
    // its instructions before track 10's turn. Its fade to end it
    // reaches 0 then, which ends it at once: a play right after starts
    // it anew, from the top, rather than reviving the faded script.
    let mut fader = vec![LOADL, 5];
    fader.extend(long(7));
    fader.extend([GETSRCDATAFIELD, 6, 1, 5, SET_LOOP, LOOP]);
    let mut busy = vec![NOP; 9_999];
    busy.push(END);
    let mut routines: Vec<&[u8]> = vec![&fader];
    routines.extend(vec![&busy[..]; 20]);
    let module = module_with(&routines, 1, 10, "kill=18,10,0,0,1\n");
    let mut player = Player::new(vec![module], samples(), 1);
    for i in 0..=20 {
        play(&mut player, &format!("e{i}"));
    }
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    play_for(&mut player, 2048, "kill");
    assert_eq!(fields_asked_in_order(&mut player), []);
    play(&mut player, "e0");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    assert_eq!(fields_asked_in_order(&mut player), []);
}

/// Waits `ms` milliseconds (loadl v1; wait v1).
fn waits(ms: u32) -> Vec<u8> {
    let mut routine = vec![LOADL, 5];
    routine.extend(long(ms));
    routine.extend([WAIT, 5]);
    routine
}

/// Asks once about field `field` (of the object its first argument
/// names), then goes on.
fn asks_once(field: u32) -> Vec<u8> {
    let mut routine = vec![LOADL, 5];
    routine.extend(long(field));
    routine.extend([GETSRCDATAFIELD, 6, 1, 5]);
    routine
}

#[test]
fn a_track_started_again_before_its_turn_restarts_itself_there() {
    // Track 10 ends track 11 and starts it again; in its turn there,
    // track 11 sets interrupt and plays itself, which starts it anew
    // from the top, so it never asks.
    let killer: &[u8] = &[LOADB_V1, 5, 11, KILL_TRK, 5, PLAY_TRK, 5, END];
    let mut restarter = vec![LOADB_V1, 55, 1, LOADB_V1, 5, 11, PLAY_TRK, 5];
    restarter.extend(asks_once(7));
    restarter.push(END);
    let module = module_of_tracks(&[(10, killer), (11, &restarter)], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e10");
    play(&mut player, "e11");
    for _ in 0..3 {
        assert_eq!(fields_asked_in_order(&mut player), []);
    }
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_held_track_let_go_during_a_block_first_runs_in_the_next() {
    // Track 0x606 asks every block; the pause holds it. Track 0x605,
    // which the pause spares, waits a second, then plays it, which lets
    // it go, and asks about field 99. A hold takes a track out of the
    // list of running tracks, so 0x606 is not in that block's copy.
    let mut releaser = waits(1000);
    releaser.extend([LOADL, 5]);
    releaser.extend(long(0x606));
    releaser.extend([PLAY_TRK, 5]);
    releaser.extend(asks_once(99));
    releaser.push(END);
    let asker = asks(7, true);
    let module = module_with(&[&releaser, &asker], 1, 0x605, "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e0");
    play(&mut player, "e1");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    player.handle(&SoundEvent::Pause, &NoSources);
    let mut blocks = Vec::new();
    while blocks.len() < 1000 && !blocks.contains(&vec![99]) {
        blocks.push(fields_asked_in_order(&mut player));
    }
    assert!(
        blocks[..blocks.len() - 1].iter().all(Vec::is_empty),
        "{blocks:?}"
    );
    assert_eq!(blocks.last(), Some(&vec![99]));
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_track_runs_from_the_module_registered_last_whichever_module_asks() {
    // Both modules have a track 10: the first's asks for field 7, the
    // second's, registered last, for 8, both with interrupt 1. Every
    // start runs the second's (0x573312, 0x57db7a), from either module's
    // event, before or after the track is killed.
    let asking = |field| {
        let mut routine = vec![LOADB_V1, 55, 1];
        routine.extend(asks(field, true));
        routine
    };
    let mut padding = vec![NOP; 400];
    padding.push(END);
    let first = module_of_tracks(&[(10, &asking(7))], "");
    let second = module_of_tracks(&[(99, &padding), (10, &asking(8))], "again=kSndobPlay,10\n");
    let mut player = Player::new(vec![first, second], samples(), 1);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [8]);
    play(&mut player, "again");
    assert_eq!(fields_asked_in_order(&mut player), [8]);
    player.handle(&SoundEvent::StopAll, &NoSources);
    player.play_from(0, 9, "e10", &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [8]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn the_expansions_register_in_their_directories_order_not_by_name() {
    // `Deluxe` registers after `ExpansionPack2` (0x414392 to 0x414458),
    // though its name sorts first and it comes first here: its track 10
    // is the one that runs, even from the other module's event. (The two
    // tracks' entry points have different track data IDs, 0x100 and
    // 0x101, so the descriptor decides which routine runs.)
    let mut deluxe = module_of_tracks(&[(10, &asks(8, false))], "");
    deluxe.name = "deluxe\\sound\\d".to_owned();
    let mut pack = module_of_tracks(
        &[(99, &[END]), (10, &asks(7, false))],
        "again=kSndobPlay,10\n",
    );
    pack.name = "expansionpack2\\sound\\e".to_owned();
    let mut player = Player::new(vec![deluxe, pack], samples(), 1);
    play(&mut player, "again");
    assert_eq!(fields_asked_in_order(&mut player), [8]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

/// A module with no tables but `[TrackData]`: each routine at its ID.
fn module_of_code(routines: &[(u32, &[u8])]) -> Module {
    let code: Vec<&[u8]> = routines.iter().map(|&(_, routine)| routine).collect();
    let (hit, addresses) = assemble(&code);
    let mut hot = String::from("[TrackData]\n");
    for (&(id, _), address) in routines.iter().zip(addresses) {
        hot.push_str(&format!("{id:#x}={address:#x}\n"));
    }
    Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap()
}

#[test]
fn a_track_starts_at_the_entry_point_registered_last() {
    // Track 10's descriptor, the second module's, registered last, names
    // track data 0x100 as its entry point. A third module, registered
    // after it and with no tracks, puts 0x100 at its own routine: set
    // interrupt, ask for field 9 once, wait. The original keeps one table
    // of track data for every module (0x57ce70), so the start runs that
    // routine (0x57dbe2), and so does the restart that a play from the
    // first module's event makes.
    let mut third = vec![LOADB_V1, 55, 1];
    third.extend(asks_once(9));
    third.extend([SET_LOOP, LOOP]);
    let first = module_of_tracks(&[(10, &asks(7, true))], "again=kSndobPlay,10\n");
    let second = module_of_tracks(&[(10, &asks(8, true))], "");
    let modules = vec![first, second, module_of_code(&[(0x100, &third)])];
    let mut player = Player::new(modules, samples(), 1);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [9]);
    assert_eq!(fields_asked_in_order(&mut player), []);
    play(&mut player, "again");
    assert_eq!(fields_asked_in_order(&mut player), [9]);
    assert_eq!(fields_asked_in_order(&mut player), []);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_call_runs_the_program_that_registered_its_target_last_and_returns() {
    // Track 10 calls track data 0x101, then asks for field 7. Its module
    // puts 0x101 at a routine that asks for 6; a module registered after
    // it, at one that asks for 8. The call runs the later one, in that
    // module's program (0x57e3d4), and its end returns to the caller's.
    let mut caller = vec![CALL];
    caller.extend(long(0x101));
    caller.extend(asks_once(7));
    caller.push(END);
    let mut own = asks_once(6);
    own.push(END);
    let mut later = asks_once(8);
    later.push(END);
    let first = module_of_tracks(&[(10, &caller), (11, &own)], "");
    let modules = vec![first, module_of_code(&[(0x101, &later)])];
    let mut player = Player::new(modules, samples(), 1);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [8, 7]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn an_expansions_hit_list_replaces_the_base_games_and_what_the_stations_added() {
    // Hit list 6 is patch 2 in the base game's table, and a station adds
    // patch 2 to it. An expansion's list 6, patch 1 alone, registers after
    // both (0x57ba20, 0x57d261), so the list is patch 1 alone.
    let choose: Vec<u8> = [
        &[5, 5][..],
        &long(6),    // loadl v1, 6
        &[68, 5],    // smart_setlist v1
        &[39, 5],    // smart_choose v1
        &[6, 50, 5], // set patch, v1
        &[2, 5, WAIT_SAMP, END],
    ]
    .concat();
    let mut base = module(&[&choose], 0);
    base.name = "sounddata\\base".to_owned();
    let mut expansion = module_with(&[&choose], 0, 10, "[GlobalHitList]\n6=1\n");
    expansion.name = "expansionpack\\more".to_owned();
    let mut player = Player::new(vec![expansion, base], samples(), 1);
    player.add_stations(&StationLists {
        hit_lists: BTreeMap::from([(6, vec![2])]),
        patches: BTreeMap::new(),
    });
    play(&mut player, "e0");
    assert_eq!(player.render(1, &NoSources)[0], 1);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_patch_plays_as_the_module_registered_last_lists_it() {
    // Both modules list patch 1; the second's, registered last, is
    // `two.xa`. The first module's track plays it.
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let first = module(&[generic], 1);
    let second = module_with(&[generic], 1, 20, "[Patch]\n1=0,two,\"two.xa\"\n");
    let mut player = Player::new(vec![first, second], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [2000, 4000, -2000]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn an_id_registered_again_as_the_other_kind_is_that_kind_only() {
    // The first module's track 10 is registered again, by a later line
    // of its own, as a patch: its event starts nothing, and that is no
    // fault (the patch's start slot returns at once, 0x57bf80). The
    // second module's track 1 replaces patch 1, so the first module's
    // track 11, which plays patch 1, plays nothing (0x57f703).
    let first = module_with(
        &[&GENERIC, &GENERIC],
        1,
        10,
        "[Patch]\n10=0,ten,\"one.xa\"\n",
    );
    let second = module_of_tracks(&[(1, &[END])], "");
    let mut player = Player::new(vec![first, second], samples(), 1);
    play(&mut player, "e0");
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    play(&mut player, "e1");
    assert_eq!(player.render(3, &NoSources), [0, 0, 0]);
    assert_eq!(player.faults, ["patch 1 is a track"]);
}

#[test]
fn a_station_patch_replaces_a_base_game_object_and_an_expansion_replaces_it() {
    // The base game's track 10 plays patch 1, `one.xa`. Station patches
    // register after the base game's tables (0x5750dc, 0x57cdd5): station
    // patch 1, `two.xa`, replaces its patch 1, and station patch 10 its
    // track 10, which then starts nothing, and no fault.
    let base = || {
        let mut base = module_with(&[&GENERIC], 1, 10, "");
        base.name = "sounddata\\base".to_owned();
        base
    };
    let stations = |ids: &[u32]| StationLists {
        hit_lists: BTreeMap::new(),
        patches: ids.iter().map(|&id| (id, "two.xa".to_owned())).collect(),
    };
    let mut player = Player::new(vec![base()], samples(), 1);
    player.add_stations(&stations(&[1]));
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [2000, 4000, -2000]);
    let mut player = Player::new(vec![base()], samples(), 1);
    player.add_stations(&stations(&[10]));
    play(&mut player, "e0");
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    // An expansion's patch 1, `one.xa` again, registers after the
    // stations and replaces station patch 1.
    let (hit, _) = assemble(&[]);
    let mut expansion =
        Module::parse("test", &hit, b"v1 5\n", b"[Patch]\n1=0,one,\"one.xa\"\n").unwrap();
    expansion.name = "expansionpack\\more".to_owned();
    let mut player = Player::new(vec![expansion, base()], samples(), 1);
    player.add_stations(&stations(&[1]));
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn stations_added_after_playing_destroy_the_objects_they_replace() {
    // Registering an ID again destroys its object (0x57ce04 to 0x57ce32).
    // Track 10 plays patch 1, `one.xa`, which is cached; station patch 1,
    // `two.xa`, replaces it while it plays, so the note stops and the
    // next start plays `two.xa`.
    let mut base = module_with(&[&GENERIC], 1, 10, "");
    base.name = "sounddata\\base".to_owned();
    let mut player = Player::new(vec![base], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(BLOCK, &NoSources)[..3], [1, 2, 3]);
    player.add_stations(&StationLists {
        hit_lists: BTreeMap::new(),
        patches: BTreeMap::from([(1, "two.xa".to_owned())]),
    });
    // The note of the patch went with it; its thread, waiting for it,
    // ends in the next block.
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    assert!(player.is_idle());
    play(&mut player, "e0");
    assert_eq!(player.render(BLOCK, &NoSources)[..3], [2000, 4000, -2000]);
    // A running track whose number a station patch takes ends with it.
    let mut base = module_of_tracks(&[(10, &asks(7, true))], "");
    base.name = "sounddata\\base".to_owned();
    let mut player = Player::new(vec![base], samples(), 1);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    player.add_stations(&StationLists {
        hit_lists: BTreeMap::new(),
        patches: BTreeMap::from([(10, "two.xa".to_owned())]),
    });
    assert!(player.is_idle());
    assert_eq!(fields_asked_in_order(&mut player), []);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_station_patch_with_the_same_sample_is_still_a_new_object() {
    // Station patch 1 names the same sample as the base game's patch 1,
    // but it is a new object (0x5750a0 to 0x5750c5), and registering it
    // destroys the old one, whose note stops.
    let mut base = module_with(&[&GENERIC], 1, 10, "");
    base.name = "sounddata\\base".to_owned();
    let mut player = Player::new(vec![base], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    player.add_stations(&StationLists {
        hit_lists: BTreeMap::new(),
        patches: BTreeMap::from([(1, "one.xa".to_owned())]),
    });
    assert_eq!(player.render(3, &NoSources), [0, 0, 0]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn destroying_a_queued_patch_leaves_the_note_it_was_queued_on() {
    // Kind 53 starts patch 1 on idle track 10, then queues patch 2 after
    // it. A station patch 2 destroys the queued patch's object, which
    // lets go of its samples (0x57fd25) but not of the voice playing
    // patch 1 (0x57e09e to 0x57e0b6): patch 1 plays to its end.
    let events = "[GlobalHitList]\n7=1\n[EventMapping]\nseqa=53,10,0,7\nseqb=53,10,0,6\n";
    let mut base = module_with(&[&GENERIC], 1, 10, events);
    base.name = "sounddata\\base".to_owned();
    let mut player = Player::new(vec![base], samples(), 1);
    play(&mut player, "seqa");
    play(&mut player, "seqb");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    player.add_stations(&StationLists {
        hit_lists: BTreeMap::new(),
        patches: BTreeMap::from([(2, "two.xa".to_owned())]),
    });
    let rest: Vec<i16> = (4..=100).chain([0]).collect();
    assert_eq!(player.render(98, &NoSources), rest);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn a_play_of_a_running_track_takes_its_own_argument_kind() {
    // One module's track 10 takes no levels from its arguments; it asks
    // once and waits, with interrupt 1. Another's, listed first, is
    // `kArgsIdVolPan`, and its event plays it for object 0, which would
    // start nothing; but the track object is the one registered last,
    // the first's, so the play restarts it.
    let mut first = vec![LOADB_V1, 55, 1];
    first.extend(asks_once(7));
    first.extend([SET_LOOP, LOOP]);
    let mut padding = vec![NOP; 400];
    padding.push(END);
    let second = module_of_kinds(
        &[(99, 2, &padding), (10, 2, &[END])],
        "again=kSndobPlay,10\n",
    );
    let modules = vec![second, module_of_kinds(&[(10, 0, &first)], "")];
    let mut player = Player::new(modules, samples(), 1);
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    assert_eq!(fields_asked_in_order(&mut player), []);
    play_for(&mut player, 0, "again");
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn seqgroup_kill_ends_a_holder_started_again_in_the_block() {
    // Track 11 asks, then holds group 1. A block later, track 10 ends it
    // and starts it again at extvol 0 (its arguments give no volume),
    // and track 12 takes group 1 before track 11's turn: the holder's
    // extvol is 0, so it ends at once and never asks again.
    let mut holder = asks_once(7);
    holder.extend(loadb(5, 1));
    holder.extend([SEQGROUP_RETURN, 5, SET_LOOP, LOOP]);
    let mut killer = waits(1);
    killer.extend([LOADB_V1, 5, 11, KILL_TRK, 5, PLAY_TRK, 5, END]);
    let mut taker = waits(1);
    taker.extend(loadb(5, 1));
    taker.extend([SEQGROUP_KILL, 5, END]);
    let module = module_of_tracks(&[(10, &killer), (11, &holder), (12, &taker)], "");
    let mut player = Player::new(vec![module], samples(), 1);
    for name in ["e10", "e12", "e11"] {
        play(&mut player, name);
    }
    assert_eq!(fields_asked_in_order(&mut player), [7]);
    assert_eq!(fields_asked_in_order(&mut player), []);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn an_unpause_runs_the_tracks_in_the_order_of_their_numbers() {
    // Track 11 started first, then 10; the unpause lets go of each, in
    // the order of the original's registry (with so few IDs, of their
    // numbers).
    let module = module_of_tracks(&[(10, &asks(10, true)), (11, &asks(11, true))], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e11");
    play(&mut player, "e10");
    assert_eq!(fields_asked_in_order(&mut player), [11, 10]);
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [10, 11]);
}

#[test]
fn an_unpause_runs_the_tracks_bucket_by_bucket() {
    // With three IDs registered the registry has 193 buckets: track 1005
    // is in bucket 40 and track 719 in 140, so 1005 goes first.
    let module = module_of_tracks(&[(719, &asks(719, true)), (1005, &asks(1005, true))], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e719");
    play(&mut player, "e1005");
    assert_eq!(fields_asked_in_order(&mut player), [719, 1005]);
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [1005, 719]);
}

#[test]
fn the_stations_register_between_the_base_tables_and_the_expansions() {
    // The base game's tables register 192 IDs (tracks 9998 and 190,
    // patch 1 and 189 more patches), then the one station patch, the
    // 193rd. The expansion's track 579 then grows the map to 389 buckets
    // before it goes in, at the head of bucket 190, ahead of track 190.
    // Had the station come last, 579 would have gone into bucket 0 of 193
    // first, and growing would have put 190 ahead of it. (Track 9998
    // takes track data 0x100, which the expansion's 579 registers again,
    // so that 190's, 0x101, is the base game's own.)
    let fillers: String = (2000..2189)
        .map(|id| format!("{id}=0,f,\"one.xa\"\n"))
        .collect();
    let mut base = module_of_tracks(
        &[(9998, &[END]), (190, &asks(190, true))],
        &format!("[Patch]\n{fillers}"),
    );
    base.name = "sounddata\\base".to_owned();
    let mut expansion = module_of_tracks(&[(579, &asks(579, true))], "");
    expansion.name = "expansionpack\\more".to_owned();
    let mut player = Player::new(vec![expansion, base], samples(), 1);
    player.add_stations(&StationLists {
        hit_lists: BTreeMap::new(),
        patches: BTreeMap::from([(FIRST_ID, "one.xa".to_owned())]),
    });
    assert_eq!(
        (player.registry().len(), player.registry().buckets()),
        (194, 389)
    );
    play(&mut player, "e190");
    play(&mut player, "e579");
    assert_eq!(fields_asked_in_order(&mut player), [190, 579]);
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [579, 190]);
}

#[test]
fn an_unpause_runs_a_buckets_tracks_newest_registered_first() {
    // Tracks 5 and 198 share bucket 5; 198, listed after 5, is at its
    // head. Track 5 started last, so it ran last before the unpause too.
    let module = module_of_tracks(&[(5, &asks(5, true)), (198, &asks(198, true))], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e198");
    play(&mut player, "e5");
    assert_eq!(fields_asked_in_order(&mut player), [198, 5]);
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [198, 5]);
    // Listed the other way round, 5 goes first.
    let module = module_of_tracks(&[(198, &asks(198, true)), (5, &asks(5, true))], "");
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e198");
    play(&mut player, "e5");
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [5, 198]);
}

#[test]
fn kind_25_inactive_blocks_kind_1_holds_kind_30_and_suppresses_unpause() {
    let looping: &[u8] = &[2, 5, WAIT_SAMP, 14];
    let events = "off=25,0\non=25,1\ntv=30,11\n";
    let module = module_with(&[looping, looping], 1, 10, events);
    let mut player = Player::new(vec![module], samples(), 1);
    assert!(player.is_active());

    // Kind 25 off marks sound inactive.
    play(&mut player, "off");
    assert!(!player.is_active());

    // Kind 1 is ignored while inactive (0x5736ee).
    play(&mut player, "e0");
    assert!(player.render(BLOCK * 2, &NoSources).iter().all(|&s| s == 0));

    // Kind 30 starts the track held and skips let_go while inactive (0x573d7b).
    play(&mut player, "tv");
    assert!(player.render(BLOCK * 2, &NoSources).iter().all(|&s| s == 0));

    // Unpause while inactive clears paused but does not release held tracks (0x573c48).
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(player.is_paused());
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert!(!player.is_paused());
    assert!(player.render(BLOCK * 2, &NoSources).iter().all(|&s| s == 0));

    // Kind 25 on resumes held tracks because the player is not paused (0x573c8e).
    play(&mut player, "on");
    assert!(player.is_active());
    assert!(player.render(BLOCK * 2, &NoSources).iter().any(|&s| s != 0));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kind_25_off_pauses_all_ambience_scores_in_live_mode_only() {
    use sims_audio::fsc::Score;
    let empty_score = Score::parse(
        b"version 7\n#\tvol\tpri\n\
          \t1024\t0\t0\t0\t1\t1\t60\t2\t1\t1\t0\t0\t0\t0\n\
          #\tvol\n\
          \t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\tNONE\n",
    )
    .unwrap();
    let events = "m3=36,3\nm0=36,0\namb0=52,0\namb1=52,1\n";
    let module = module_with(&[&[END]], 1, 10, events);
    let mut player = Player::new(vec![module], samples(), 1);
    assert!(player.add_score(0, &empty_score, "a"));
    assert!(player.add_score(1, &empty_score, "b"));
    play(&mut player, "amb0");

    // In initial music mode 4 (non-live), kind 25 off leaves every score unpaused.
    player.set_active(false, &NoSources);
    assert!(!player.score(0).unwrap().is_paused());
    assert!(!player.score(1).unwrap().is_paused());
    player.set_active(true, &NoSources);

    // Enter live music mode (mode 3 then mode 0).
    let mut day = Snapshot::default();
    day.absent[4] = 12;
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "m3".into(),
        },
        &day,
    );
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "m0".into(),
        },
        &day,
    );
    assert!(!player.score(0).unwrap().is_paused());
    assert!(!player.score(1).unwrap().is_paused());

    // In live mode, kind 25 off pauses EVERY ambience score (+0x104..+0x110),
    // including the non-current score 1.
    player.set_active(false, &day);
    assert!(player.score(0).unwrap().is_paused());
    assert!(player.score(1).unwrap().is_paused());

    // Kind 25 on while paused does NOT unpause either score (0x573c8e).
    player.handle(&SoundEvent::Pause, &day);
    player.set_active(true, &day);
    assert!(player.score(0).unwrap().is_paused());
    assert!(player.score(1).unwrap().is_paused());

    // Unpause, then pause both scores again via kind 25 off while unpaused;
    // kind 25 on while unpaused resumes ONLY the current score (+0x100,
    // 0x573ca1), leaving non-current score 1 paused.
    player.handle(&SoundEvent::Unpause, &day);
    assert!(!player.score(0).unwrap().is_paused());
    assert!(player.score(1).unwrap().is_paused());
    player.set_active(false, &day);
    assert!(player.score(0).unwrap().is_paused());
    assert!(player.score(1).unwrap().is_paused());
    player.set_active(true, &day);
    assert!(!player.score(0).unwrap().is_paused());
    assert!(player.score(1).unwrap().is_paused());

    // Switching to mode 1 via kind 52 applies the manager's pause state (0x573fd4).
    play(&mut player, "amb1");
    assert!(!player.score(1).unwrap().is_paused());
}

#[test]
fn kind_25_on_in_neighbourhood_mode_unpauses_even_when_paused_and_lets_main_music_go() {
    use sims_audio::player::music::MAIN_MUSIC_LOOP;
    // Track 10 waits 1000 ms then asks field 10; track 515 (MainMusicLoop)
    // waits 1000 ms then asks field 11.
    let mut t10 = waits(1000);
    t10.extend(asks(10, false));
    let mut t515 = waits(1000);
    t515.extend(asks(11, false));
    let module = module_of_tracks(
        &[(10, &t10), (MAIN_MUSIC_LOOP, &t515)],
        "[EventMapping]\nm3=36,3\n",
    );
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, "e10");
    play(&mut player, &format!("e{MAIN_MUSIC_LOOP}"));
    // Start both waits, then pause.
    assert_eq!(fields_asked_in_order(&mut player), []);
    player.handle(&SoundEvent::Pause, &NoSources);
    assert!(player.is_paused());

    // While paused in mode 4, kind 25 on skips 0x574904 (track 10 stays held)
    // but still calls 0x575155, ending MainMusicLoop's wait (0x573cc0).
    player.set_active(true, &NoSources);
    assert!(player.is_paused());
    assert_eq!(fields_asked_in_order(&mut player), [11]);

    // In mode 3 (neighbourhood), kind 25 on calls 0x574904 even while
    // paused (0x573cb2), clearing pause and releasing track 10.
    play(&mut player, "m3");
    assert!(player.is_paused());
    player.set_active(true, &NoSources);
    assert!(!player.is_paused());
    assert_eq!(fields_asked_in_order(&mut player), [10]);
}

#[test]
fn unpause_lets_main_music_loop_go_last() {
    use sims_audio::player::music::MAIN_MUSIC_LOOP;
    // With 193 buckets, 515 % 193 = 129 and 520 % 193 = 134, so registry
    // order puts 515 BEFORE 520; 0x575155 at the end of 0x574904 lets
    // MainMusicLoop (515) go again, moving it LAST.
    let module = module_of_tracks(
        &[(MAIN_MUSIC_LOOP, &asks(1, true)), (520, &asks(2, true))],
        "",
    );
    let mut player = Player::new(vec![module], samples(), 1);
    play(&mut player, &format!("e{MAIN_MUSIC_LOOP}"));
    play(&mut player, "e520");
    assert_eq!(fields_asked_in_order(&mut player), [1, 2]);
    player.handle(&SoundEvent::Unpause, &NoSources);
    assert_eq!(fields_asked_in_order(&mut player), [2, 1]);
}

#[test]
fn kind_25_volume_nudge_adjusts_placement_for_100_updates_then_stops() {
    use sims_audio::placement;
    // A constant loud sample so extvol = 1 vs 0 and 749 vs 750 are
    // directly observable in the rendered PCM.
    let loud_samples = || {
        let mut m = samples();
        m.insert(
            "one.xa".to_owned(),
            Pcm {
                rate: RATE,
                channels: 1,
                samples: vec![16_384; RATE as usize],
            },
        );
        m
    };
    let looping: &[u8] = &[2, 5, WAIT_SAMP, LOOP];
    let day_at = |x: i32| {
        let mut s = on_lot(&[(9, x)]);
        s.absent[4] = 12;
        s
    };
    let right = day_at(800); // vol = 750 -> nudged 749
    let offscreen = day_at(5000); // vol = 0 -> nudged 1
    // Find a screen x whose placement volume is exactly 1 (nudged to 0).
    let vol_one_x = (400..5000)
        .find(|&x| {
            placement::combine(&[9], sims_audio::player::SPL_NORMAL, &day_at(x))
                .is_ok_and(|(v, _)| v == 1)
        })
        .expect("a screen x with placement volume 1");
    let barely = day_at(vol_one_x);

    let mut p_right = Player::new(vec![module_with(&[looping], 1, 10, "")], loud_samples(), 1);
    let mut p_zero = Player::new(vec![module_with(&[looping], 1, 10, "")], loud_samples(), 1);
    let mut p_one = Player::new(vec![module_with(&[looping], 1, 10, "")], loud_samples(), 1);

    play_on(&mut p_right, 9, &right);
    play_on(&mut p_zero, 9, &offscreen);
    play_on(&mut p_one, 9, &barely);
    let _ = p_right.render_stereo(BLOCK + 1, &right);
    let _ = p_zero.render_stereo(BLOCK + 1, &offscreen);
    let _ = p_one.render_stereo(BLOCK + 1, &barely);

    // Before kind 25: camera_moved writes exact vol (750, 0, 1).
    p_right.camera_moved(0, 12, &right);
    let normal_right = p_right.render_stereo(1, &right);
    p_zero.camera_moved(0, 12, &offscreen);
    assert_eq!(p_zero.render_stereo(1, &offscreen), [0, 0]);
    p_one.camera_moved(0, 12, &barely);
    let normal_one = p_one.render_stereo(1, &barely);
    assert!(normal_one[0] > 0 && normal_one[1] > 0, "{normal_one:?}");

    // Kind 25 on arms the 100-update countdown (+0x114 = 100), after
    // restoring normal vol.
    p_right.set_active(true, &right);
    assert_eq!(p_right.render_stereo(1, &right), normal_right);
    p_zero.set_active(true, &offscreen);
    assert_eq!(p_zero.render_stereo(1, &offscreen), [0, 0]);
    p_one.set_active(true, &barely);
    assert_eq!(p_one.render_stereo(1, &barely), normal_one);

    // Mid-block camera_moved while countdown is nonzero (0x574701):
    // - vol 750 -> 749 (strictly quieter than normal);
    // - vol 0 -> 1 (strictly nonzero!);
    // - vol 1 -> 0 (silent!).
    p_right.camera_moved(0, 12, &right);
    let nudged_right = p_right.render_stereo(1, &right);
    assert!(
        nudged_right[0] < normal_right[0] && nudged_right[1] < normal_right[1],
        "{nudged_right:?} vs {normal_right:?}"
    );
    p_zero.camera_moved(0, 12, &offscreen);
    let nudged_zero = p_zero.render_stereo(1, &offscreen);
    assert!(nudged_zero[0] > 0 && nudged_zero[1] > 0, "{nudged_zero:?}");
    p_one.camera_moved(0, 12, &barely);
    assert_eq!(p_one.render_stereo(1, &barely), [0, 0]);

    // At each 20 ms timer boundary (`TIMER_FRAMES` = 441 frames at 22,050 Hz,
    // `0x572fc3` -> `0x573019`), the countdown restores normal vol and
    // decrements `+0x114` (100 updates = 2.00 s). Advance 99 timer ticks
    // (1 remains -> still nudging), then 1 more timer tick (0 remains ->
    // nudging stops).
    let timer = sims_audio::player::TIMER_FRAMES;
    let to_next = timer - (p_zero.frame() as usize % timer);
    let _ = p_zero.render_stereo(to_next + 98 * timer + 1, &offscreen);
    p_zero.camera_moved(0, 12, &offscreen);
    assert!(p_zero.render_stereo(1, &offscreen)[0] > 0);
    let to_next = timer - (p_zero.frame() as usize % timer);
    let _ = p_zero.render_stereo(to_next + 1, &offscreen);
    p_zero.camera_moved(0, 12, &offscreen);
    assert_eq!(p_zero.render_stereo(1, &offscreen), [0, 0]);
}

#[test]
fn day_and_night_loop_picks_nite_loop_beach_loop_and_skips_downtown() {
    use sims_audio::fsc::Score;
    use sims_audio::player::{BEACH_LOOP, NITE_LOOP};

    let empty_score = Score::parse(
        b"version 7\n#\tvol\tpri\n\
          \t1024\t0\t0\t0\t1\t1\t60\t2\t1\t1\t0\t0\t0\t0\n\
          #\tvol\n\
          \t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\tNONE\n",
    )
    .unwrap();
    // Each loop track plays a distinct constant sample so we can tell
    // which one is sounding (nite_loop: patch 1 = +4000; BeachLoop: patch 2 = -8000).
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    map.insert(
        "two.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![-8000; RATE as usize],
        },
    );
    let looping: &[u8] = &[2, 5, WAIT_SAMP, 14];
    let (hit, addresses) = assemble(&[looping, looping]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [Track]\n\
         {NITE_LOOP}=0,nite_loop,1024,0,0,1,0,0x100,1\n\
         {BEACH_LOOP}=0,BeachLoop,1024,0,0,1,0,0x101,2\n\
         [EventMapping]\n\
         m3=36,3\n\
         m0=36,0\n\
         amb0=52,0\n\
         amb1=52,1\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], map, 1);
    assert!(player.add_score(0, &empty_score, "a"));
    assert!(player.add_score(1, &empty_score, "b"));

    let snap = |hour: i32, lot: i16| {
        let mut s = Snapshot::default();
        s.absent[4] = hour;
        s.lot = lot;
        s
    };

    // Outside live mode (mode 4, then mode 3): night starts nothing.
    let night = snap(22, 1);
    player.camera_moved(0, 22, &night);
    assert!(
        player
            .render_stereo(BLOCK * 4, &night)
            .iter()
            .all(|&s| s == 0)
    );
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "m3".into(),
        },
        &night,
    );
    assert!(
        player
            .render_stereo(BLOCK * 4, &night)
            .iter()
            .all(|&s| s == 0)
    );

    // Enter live mode at daytime (hours 6..=18): still silent.
    let day6 = snap(6, 1);
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "m0".into(),
        },
        &day6,
    );
    assert!(
        player
            .render_stereo(BLOCK * 4, &day6)
            .iter()
            .all(|&s| s == 0)
    );
    let day18 = snap(18, 1);
    player.camera_moved(0, 18, &day18);
    assert!(
        player
            .render_stereo(BLOCK * 4, &day18)
            .iter()
            .all(|&s| s == 0)
    );

    // Hour 19 (> 18) on lot 1: nite_loop starts and fades to VOLUME_STEPS[fx_step]
    // (0x574a07..0x574a10; step 3 -> 490).
    player.set_fx_volume_step(3, &day18);
    let night19 = snap(19, 1);
    player.camera_moved(0, 19, &night19);
    let out_half = player.render_stereo(BLOCK * 4, &night19);
    assert!(out_half.iter().any(|&s| s > 0), "{out_half:?}");
    // Fade target was 490 (and group 1 is 490, so logarithmic attenuation
    // scales 4000 far below full volume).
    let peak_half = *out_half.iter().max().unwrap();
    assert!(peak_half > 0 && peak_half < 400, "{peak_half}");

    // Downtown lots 20..=29 silence nite_loop (0x57494e..0x574956).
    let downtown20 = snap(5, 20);
    player.camera_moved(0, 5, &downtown20);
    let _ = player.render_stereo(BLOCK * 16, &downtown20);
    assert!(
        player
            .render_stereo(BLOCK * 4, &downtown20)
            .iter()
            .all(|&s| s == 0),
        "lot 20 should silence nite_loop"
    );
    // Lot 19 and lot 30 are outside Downtown, so hour 5 plays nite_loop again.
    player.set_fx_volume_step(10, &downtown20);
    let lot30 = snap(5, 30);
    player.camera_moved(0, 5, &lot30);
    let out_full = player.render_stereo(BLOCK * 4, &lot30);
    assert!(*out_full.iter().max().unwrap() > 3500);

    // Kind 52 (amb1 -> Beach) alone does NOT call 0x574978, so nite_loop
    // keeps playing until camera_moved switches to BeachLoop (even at noon
    // on a Downtown lot!).
    let downtown_noon = snap(12, 25);
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "amb1".into(),
        },
        &downtown_noon,
    );
    assert!(
        player
            .render_stereo(BLOCK * 4, &downtown_noon)
            .iter()
            .any(|&s| s > 0)
    );
    player.camera_moved(0, 12, &downtown_noon);
    // Let nite_loop finish fading out (11 blocks at speed 100); BeachLoop plays (-8000).
    let _ = player.render_stereo(BLOCK * 16, &downtown_noon);
    let beach = player.render_stereo(BLOCK * 4, &downtown_noon);
    assert!(*beach.iter().min().unwrap() < -7000, "{beach:?}");

    // Pause stops the loop (0x574844); unpause while active restarts it (0x574918).
    player.handle(&SoundEvent::Pause, &downtown_noon);
    assert!(
        player
            .render_stereo(BLOCK * 4, &downtown_noon)
            .iter()
            .all(|&s| s == 0)
    );
    player.handle(&SoundEvent::Unpause, &downtown_noon);
    // Note: BeachLoop was mid-fade-out when paused, so let that fade finish,
    // then the next camera_moved starts it fresh.
    let _ = player.render_stereo(BLOCK * 16, &downtown_noon);
    player.camera_moved(0, 12, &downtown_noon);
    let resumed = player.render_stereo(BLOCK * 4, &downtown_noon);
    assert!(*resumed.iter().min().unwrap() < -7000);

    // Inactive sound stops the loop on 0x574978 (0x5749a1).
    player.set_active(false, &downtown_noon);
    player.camera_moved(0, 12, &downtown_noon);
    let _ = player.render_stereo(BLOCK * 16, &downtown_noon);
    assert!(
        player
            .render_stereo(BLOCK * 4, &downtown_noon)
            .iter()
            .all(|&s| s == 0)
    );
    // Only the music tracks this hand-built module does not define.
    assert_eq!(
        player.faults,
        ["no track 517", "no track 2957", "no track 519"]
    );
}

#[test]
fn day_and_night_loop_ignores_a_patch_object_without_fault() {
    use sims_audio::player::NITE_LOOP;
    let (hit, _) = assemble(&[&[END]]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [Patch]\n\
         {NITE_LOOP}=0,p,\"one.xa\"\n\
         [EventMapping]\n\
         m3=36,3\n\
         m0=36,0\n"
    );
    let module = Module::parse("test", &hit, b"", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], samples(), 1);
    let night = Snapshot::default(); // hour 0
    for name in ["m3", "m0"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &night,
        );
    }
    player.camera_moved(0, 0, &night);
    assert!(
        player
            .render_stereo(BLOCK * 4, &night)
            .iter()
            .all(|&s| s == 0)
    );
    // Only the music tracks this module lacks; no fault for NITE_LOOP.
    assert_eq!(
        player.faults,
        ["no track 517", "no track 2957", "no track 519"]
    );
}

#[test]
fn kind_105_places_remaining_zero_object_instances_again() {
    let mut map = samples();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    // Starts a note, then zeroes its own extvol (`loadb 19, 0`) and waits
    // so it is silent until re-placed.
    let mut silent_loop = vec![2, 5];
    silent_loop.extend(loadb(sims_audio::player::EXTVOL, 0));
    silent_loop.extend([WAIT_SAMP, 14]);
    let module = module_with(&[&silent_loop], 1, 10, "tv=30,10\nkill_objs=105,0\n");
    let mut player = Player::new(vec![module], map, 1);
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "tv".into(),
        },
        &NoSources,
    );
    assert!(
        player
            .render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .all(|&s| s == 0)
    );
    // Kind 105 keeps object 0 and re-places it via 0x574647 -> 0x57447f,
    // restoring extvol to FULL (1024) so the note sounds again.
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "kill_objs".into(),
        },
        &NoSources,
    );
    assert!(
        player
            .render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .any(|&s| s != 0)
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn muted_group_refuses_start_so_beach_loop_starts_after_unmuting() {
    use sims_audio::fsc::Score;
    use sims_audio::player::BEACH_LOOP;

    let empty_score = Score::parse(
        b"version 7\n#\tvol\tpri\n\
          \t1024\t0\t0\t0\t1\t1\t60\t2\t1\t1\t0\t0\t0\t0\n\
          #\tvol\n\
          \t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\tNONE\n",
    )
    .unwrap();
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    let looping: &[u8] = &[2, 5, WAIT_SAMP, 14];
    let (hit, addresses) = assemble(&[looping, looping]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         {BEACH_LOOP}=0,BeachLoop,1024,0,0,1,0,0x100,1\n\
         20=0,ungrouped,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         m3=36,3\n\
         m0=36,0\n\
         amb1=52,1\n\
         e20=1,20\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], map, 1);
    assert!(player.add_score(0, &empty_score, "a"));
    assert!(player.add_score(1, &empty_score, "b"));
    let mut day = Snapshot::default();
    day.absent[4] = 12;

    // Mute group 1 (effects) and enter Beach live mode: 0x57d969 refuses
    // BeachLoop's start instead of leaving it stuck at extvol = 0.
    player.set_group_volume(1, 0);
    for name in ["m3", "amb1", "m0"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &day,
        );
    }
    player.camera_moved(0, 12, &day);
    assert!(
        player
            .render_stereo(BLOCK * 4, &day)
            .iter()
            .all(|&s| s == 0)
    );

    // Unmuting group 1 and running kind 33 starts BeachLoop fresh and audible.
    player.set_group_volume(1, 1024);
    player.camera_moved(0, 12, &day);
    let out = player.render_stereo(BLOCK * 4, &day);
    assert!(*out.iter().max().unwrap() > 3500, "{out:?}");

    // Group 0 is never refused even when group_volumes[0] is 0 (0x57d979):
    // test on a fresh player so BeachLoop cannot mask the result.
    let mut map2 = BTreeMap::new();
    map2.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    let module2 = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut solo = Player::new(vec![module2], map2, 1);
    solo.set_group_volume(0, 0);
    solo.handle(
        &SoundEvent::Play {
            object: 0,
            name: "e20".into(),
        },
        &day,
    );
    assert!(solo.render_stereo(BLOCK * 2, &day).iter().all(|&s| s == 0));
    solo.set_group_volume(0, 1024);
    // Track 20 was already started while group 0 was 0, so raising its
    // volume makes it audible without a second start.
    assert!(
        solo.render_stereo(BLOCK * 2, &day)
            .iter()
            .any(|&s| s > 3500)
    );
}

#[test]
fn muted_kind_1_still_places_and_releases_an_already_held_track() {
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    let looping: &[u8] = &[2, 5, WAIT_SAMP, 14];
    let (hit, addresses) = assemble(&[looping]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,sfx_loop,1024,2,0,1,0,0x100,1\n\
         [EventMapping]\n\
         e0=1,10\n",
        addresses[0]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], map, 1);
    let center = on_lot(&[(9, 400)]);
    let right = on_lot(&[(9, 800)]);

    // Start track 10 (group 1) at center, then hold it with a pause.
    play_on(&mut player, 9, &center);
    assert!(
        player
            .render_stereo(BLOCK * 2, &center)
            .iter()
            .any(|&s| s > 3500)
    );
    player.handle(&SoundEvent::Pause, &center);
    assert!(
        player
            .render_stereo(BLOCK * 2, &center)
            .iter()
            .all(|&s| s == 0)
    );

    // Mute group 1, then play track 10 again at the right edge while still
    // paused: start() returns early at 0x57d9a5, but kind 1 still places it
    // (0x57379a..0x5737a4) and calls let_go (0x5737ab -> 0x57dc68), clearing
    // its hold.
    player.set_group_volume(1, 0);
    play_on(&mut player, 9, &right);
    assert!(
        player
            .render_stereo(BLOCK * 2, &right)
            .iter()
            .all(|&s| s == 0)
    );

    // Unmuting group 1 (without unpausing!) makes the released track audible
    // immediately at its updated right-edge placement (right > left).
    player.set_group_volume(1, 1024);
    let out = player.render_stereo(BLOCK * 2, &right);
    let [left, right_ch] = [out[out.len() - 2], out[out.len() - 1]];
    assert!(left > 0 && right_ch > left, "{left} {right_ch}");
}

#[test]
fn muted_replay_places_and_releases_a_held_note_fading_after_end() {
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    // Script that reaches `end` while its note plays, continuing into an
    // explicit wait loop while fading out (0x57e3ee → 0x57dca3).
    let one_shot: &[u8] = &[2, 5, END];
    let mut hold_pc = vec![SET_LOOP];
    hold_pc.extend(loadl(5, 1000));
    hold_pc.extend([WAIT, 5, LOOP]);
    let (hit, addresses) = assemble(&[one_shot, &hold_pc]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,sfx_oneshot,1024,2,0,1,0,0x100,1\n\
         [EventMapping]\n\
         e0=1,10\n",
        addresses[0]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], map, 1);
    let center = on_lot(&[(9, 400)]);
    let right = on_lot(&[(9, 800)]);

    // Start track 10 (group 1) at center; `end` starts its fade while
    // keeping it running, then pause holds it mid-fade.
    play_on(&mut player, 9, &center);
    assert!(
        player
            .render_stereo(BLOCK * 2, &center)
            .iter()
            .any(|&s| s > 2000)
    );
    player.handle(&SoundEvent::Pause, &center);
    assert!(
        player
            .render_stereo(BLOCK * 2, &center)
            .iter()
            .all(|&s| s == 0)
    );

    // Mute group 1, then replay track 10 at the right edge while paused:
    // start() returns early at 0x57d9a5, yet kind 1 still places the
    // running fading track and clears its hold at 0x57dc68.
    player.set_group_volume(1, 0);
    play_on(&mut player, 9, &right);
    assert!(
        player
            .render_stereo(BLOCK * 2, &right)
            .iter()
            .all(|&s| s == 0)
    );

    // Unmuting group 1 (without global unpause) makes the held note resume
    // immediately at its updated right-edge placement (right > left).
    player.set_group_volume(1, 1024);
    let out = player.render_stereo(BLOCK * 2, &right);
    let [left, right_ch] = [out[out.len() - 2], out[out.len() - 1]];
    assert!(left > 0 && right_ch > left, "{left} {right_ch}");
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn muted_replay_keeps_fading_spl_releases_music_and_leaves_idle_group_levels() {
    // 1) Fading track after `end` with high `Spl` (10001): muted replay
    //    keeps that `Spl` (0x57314b) instead of falling back to 20, so
    //    an off-screen object stays audible after unmuting without
    //    needing a camera update.
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    // `loadl 57 10001; note_on v1; end` followed by an explicit wait loop.
    let mut spl_script = vec![5, 57];
    spl_script.extend_from_slice(&10001i32.to_le_bytes());
    spl_script.extend_from_slice(&[2, 5, END]);
    let mut hold_pc = vec![SET_LOOP];
    hold_pc.extend(loadl(5, 1000));
    hold_pc.extend([WAIT, 5, LOOP]);
    // Replacement `river_loop` (track 2957, group 1): `note_on v1; end`
    // followed by an explicit wait loop.
    let one_shot: &[u8] = &[2, 5, END];
    let (hit, addresses) = assemble(&[&spl_script, &hold_pc, one_shot, &hold_pc]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x200={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [GlobalHitList]\n\
         20=1\n\
         [Track]\n\
         10=0,sfx_spl,1024,2,0,1,0,0x100,1\n\
         2957=0,river_loop,1024,0,0,1,0,0x200,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         r=1,2957\n\
         m3=36,3\n\
         seq=53,2957,0,20\n",
        addresses[0], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module.clone()], map.clone(), 1);
    let center = on_lot(&[(9, 400)]);
    let far = on_lot(&[(9, 5000)]);

    play_on(&mut player, 9, &center);
    assert!(
        player
            .render_stereo(BLOCK * 2, &center)
            .iter()
            .any(|&s| s > 2000)
    );
    player.handle(&SoundEvent::Pause, &center);
    player.set_group_volume(1, 0);
    play_on(&mut player, 9, &far);
    player.set_group_volume(1, 1024);
    assert!(
        player.render_stereo(BLOCK * 2, &far).iter().any(|&s| s > 0),
        "fading track's Spl (10001) should survive muted replay"
    );

    // 2) Music-mode start (`change_music`) unconditionally clears hold on a
    //    muted `river_loop` (2957) fading after `end` (`0x57523f -> 0x57dc68`).
    let mut music = Player::new(vec![module.clone()], map.clone(), 1);
    play(&mut music, "r");
    assert!(
        music
            .render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .any(|&s| s > 2000)
    );
    music.handle(&SoundEvent::Pause, &NoSources);
    assert!(
        music
            .render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .all(|&s| s == 0)
    );
    music.set_group_volume(1, 0);
    play(&mut music, "m3");
    music.set_group_volume(1, 1024);
    assert!(
        music
            .render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .any(|&s| s > 1000),
        "muted music-mode start should still clear river_loop's hold"
    );

    // 3) Group volume change (`0x57c0ea`) walks the running-track list and
    //    does not refresh idle kind-53 notes.
    let mut idle = Player::new(vec![module], map, 1);
    play(&mut idle, "seq");
    assert!(
        idle.render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .any(|&s| s > 3500)
    );
    idle.set_group_volume(1, 0);
    assert!(
        idle.render_stereo(BLOCK * 2, &NoSources)
            .iter()
            .any(|&s| s > 3500),
        "group volume change should leave an idle kind-53 voice at its started gain"
    );
}

#[test]
fn repeated_kind_30_updates_running_note_gain_immediately() {
    let mut map = samples();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    let looping: &[u8] = &[2, 5, WAIT_SAMP, 14];
    let module = module_with(&[looping], 1, 10, "tv=30,10\n");
    let mut player = Player::new(vec![module], map, 1);
    let center = on_lot(&[(9, 400)]);
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "tv".into(),
        },
        &center,
    );
    // Advance 1 frame into a block so the note is sounding at full volume.
    let before = player.render_stereo(BLOCK + 1, &center);
    assert_eq!(&before[before.len() - 2..], &[4000, 4000]);

    // Repeat kind 30 mid-block with the object now off-screen: the very
    // next sample drops to 0 immediately (0x573d64 -> 0x57df13).
    let offscreen = on_lot(&[(9, 5000)]);
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "tv".into(),
        },
        &offscreen,
    );
    assert_eq!(player.render_stereo(1, &offscreen), [0, 0]);
}

#[test]
fn day_and_night_loop_handles_script_that_ends_while_its_note_plays() {
    use sims_audio::fsc::Score;
    use sims_audio::player::NITE_LOOP;

    let empty_score = Score::parse(
        b"version 7\n#\tvol\tpri\n\
          \t1024\t0\t0\t0\t1\t1\t60\t2\t1\t1\t0\t0\t0\t0\n\
          #\tvol\n\
          \t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\tNONE\n",
    )
    .unwrap();
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    // Replacement loop script: `note_on v1; end` followed by an explicit
    // wait (stays running while fading out at speed 100, 0x57dca3).
    // On start, `update_day_night` starts `extvol` at 0 and fades it up
    // to 1024 at speed 5000 (hits 1024 on block 0 before `note_on`),
    // then `end` starts fading `extvol` down from 1024 at speed 100.
    let one_shot: &[u8] = &[2, 5, END];
    let mut hold_pc = loadl(5, 1000);
    hold_pc.extend([WAIT, 5]);
    let (hit, addresses) = assemble(&[one_shot, &hold_pc]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         {NITE_LOOP}=0,nite_loop,1024,0,0,1,0,0x100,1\n\
         [EventMapping]\n\
         m3=36,3\n\
         m0=36,0\n",
        addresses[0]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], map, 1);
    assert!(player.add_score(0, &empty_score, "a"));
    let mut night = Snapshot::default();
    night.absent[4] = 22;
    let mut day = Snapshot::default();
    day.absent[4] = 12;

    for name in ["m3", "m0"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &night,
        );
    }
    let first = player.render_stereo(BLOCK * 2, &night);
    let [l1, r1] = [first[first.len() - 2], first[first.len() - 1]];
    assert!(l1 > 0 && l1 < 4000 && l1 == r1, "{l1} {r1}");

    // Calling camera_moved again at night while the fade is still running
    // must NOT start a second overlapping note (`day_night_playing` sees
    // the running thread, 0x574a00).
    player.camera_moved(0, 22, &night);
    let second = player.render_stereo(BLOCK * 2, &night);
    let [l2, r2] = [second[second.len() - 2], second[second.len() - 1]];
    assert!(l2 > 0 && l2 < l1 && l2 == r2, "before {l1}, after {l2}");

    // Switching to daytime stops the fading loop once its fade finishes.
    player.camera_moved(0, 12, &day);
    let _ = player.render_stereo(BLOCK * 12, &day);
    assert!(
        player
            .render_stereo(BLOCK * 2, &day)
            .iter()
            .all(|&s| s == 0)
    );
}

#[test]
fn end_and_return_while_a_note_plays_fade_out_at_speed_100_and_run_on() {
    // Both `end` (opcode 12) and `return` (opcode 8) jump to 0x57e3ee:
    // with an empty call stack and a playing note, they call 0x57dca3
    // (fade extvol to 0 at speed 100, kill_after_fade = true), keep the
    // track running, and continue at `next` in the SAME block (0x57d8b3).
    use sims_audio::levels::{Inputs, levels};
    const RETURN: u8 = 8;
    const IFGT: u8 = 64;

    let expected_sample = |extvol: i32| -> i16 {
        let g = levels(&Inputs {
            vol: 1024,
            extvol,
            pan: 512,
            pitch: 3600,
            duck_priority: 0,
            three_d: false,
            group_volume: 1024,
            main_duck_priority: 0,
            master: 1024,
        })
        .gains[0];
        ((10_000i64 * i64::from(g)) >> 16) as i16
    };

    for opcode in [END, RETURN] {
        // Routine 0 (0x100): `note_on v1; loadl 5, 0; <end/return>`
        // Immediately followed by `ifgt 0x101; loadl 5, 1000; wait 5`.
        // `loadl 5, 0` clears flags to 0, then `0x57dca3` writes FADE_ON=1
        // last (0x57dd05 -> 0x57de98), setting positive flags so the run-on
        // `ifgt` jumps to 0x101 in the VERY SAME block (block 0), where it
        // queries object 9 with field 42 before yielding via `wait`.
        let mut r0 = NOTE_ON_V1.to_vec();
        r0.extend(loadl(5, 0));
        r0.push(opcode);
        r0.push(IFGT);
        r0.extend(long(0x101));
        r0.extend(loadl(5, 1000));
        r0.extend([WAIT, 5]);
        let mut r1 = loadl(7, 42);
        r1.extend([72, 6, 1, 7]); // getsrcdatafield v2, arg1, v3 (42)
        r1.extend(loadl(5, 1000));
        r1.extend([WAIT, 5]);
        let probe = Probe {
            lot: on_lot(&[(9, 400)]),
            asked: Default::default(),
        };
        let mut player = Player::new(vec![module(&[&r0, &r1], 1)], long_sample("one.xa"), 1);
        play_on(&mut player, 9, &probe.lot);

        // Block 0: note plays at extvol 1024, and the run-on `ifgt` MUST
        // have executed during block 0 itself (before block 1's fade step).
        let b0 = player.render(BLOCK, &probe);
        assert_eq!(b0[0], expected_sample(1024), "op {opcode} block 0");
        assert_eq!(
            *probe.asked.borrow(),
            [(9, 42)],
            "op {opcode}: run-on branch must execute in block 0"
        );
        assert!(!player.is_idle(), "op {opcode}");

        // Blocks 1..=10: extvol steps by EXACTLY 100 each block
        // (924, 824, ..., 24).
        for step_idx in 1..=10 {
            let extvol = 1024 - 100 * step_idx;
            let block = player.render(BLOCK, &probe);
            assert_eq!(
                block[0],
                expected_sample(extvol),
                "op {opcode} block {step_idx} (extvol {extvol})"
            );
        }
        // Block 11: extvol hits 0, FADE_ON becomes 0, kill_after_fade kills
        // the track and cuts its note.
        let b11 = player.render(BLOCK, &probe);
        assert!(b11.iter().all(|&s| s == 0), "op {opcode}: {b11:?}");
        assert!(player.is_idle(), "op {opcode}");
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn end_inside_a_call_pops_the_stack_without_fading_the_note() {
    // When the return stack is non-empty, `end` pops the caller's address
    // (0x57e3ee..0x57e428) and does NOT trigger 0x57dca3's fade.
    const CALL: u8 = 7;
    let mut caller = vec![CALL];
    caller.extend(long(0x101));
    caller.extend(loadl(5, 1000));
    caller.extend([WAIT, 5]);
    let sub: &[u8] = &[NOTE_ON_V1[0], NOTE_ON_V1[1], END];
    let mut player = Player::new(vec![module(&[&caller, sub], 1)], long_sample("one.xa"), 1);
    play(&mut player, "e0");
    // Render 15 blocks (> 11 blocks of a speed-100 fade): note stays at
    // full volume (10000) the entire time.
    let out = player.render(15 * BLOCK, &NoSources);
    assert_eq!(out[out.len() - 1], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn replay_during_end_fade_cancels_fade_or_restarts_when_interrupt_is_set() {
    // Without `interrupt`, replaying a track while it is fading after
    // `end` refreshes it and clears the fade (0x57d9b2..0x57d9bb) with
    // one note still playing. With `interrupt = 1`, it restarts in place
    // and cuts the old note (0x57da26 -> 0x57f625).
    for interrupt in [false, true] {
        let mut r0 = Vec::new();
        if interrupt {
            r0.extend(loadb(55, 1));
        }
        r0.extend(NOTE_ON_V1);
        r0.push(END);
        let mut hold_pc = vec![SET_LOOP];
        hold_pc.extend(loadl(5, 1000));
        hold_pc.extend([WAIT, 5, LOOP]);
        let mut player = Player::new(vec![module(&[&r0, &hold_pc], 1)], long_sample("one.xa"), 1);
        play(&mut player, "e0");
        // Advance 3 blocks into the fade (extvol = 1024 -> 924 -> 824).
        let before = player.render(3 * BLOCK, &NoSources);
        let faded = before[before.len() - 1];
        assert!(faded > 0 && faded < 10_000, "{faded}");

        // Replay mid-fade:
        play(&mut player, "e0");
        if !interrupt {
            // Fade is cancelled; the single note stays at full placed
            // volume (10000) well past 11 blocks (no second note = 20000).
            let after = player.render(15 * BLOCK, &NoSources);
            assert_eq!(after[after.len() - 1], 10_000);
            assert!(!player.is_idle());
        } else {
            // Restarted in place: fresh start resets extvol to 1024 and
            // starts a fresh fade after running `note_on; end` again,
            // finishing cleanly after 11 more blocks.
            let after_one = player.render(BLOCK, &NoSources);
            assert_eq!(after_one[0], 10_000);
            let _ = player.render(11 * BLOCK, &NoSources);
            assert!(player.is_idle());
        }
        assert!(player.faults.is_empty(), "{:?}", player.faults);
    }
}

#[test]
fn end_while_a_note_plays_with_target_fades_target_and_kills_self_next_block() {
    // `0x57dca3` writes FADE_DEST/VAR/SPEED/ON through `target` (vtable
    // +0x84, 0x57de57), while setting `kill_after_fade` (+0x1cf) on
    // itself. Track 10 plays a note, points `target` at track 11, and
    // hits `end` (continuing into a wait). On the next block track 10's
    // own FADE_ON is 0 with kill_after_fade set, so track 10 kills
    // itself (0x57d86e -> 0x57dd3e), while track 11 fades its extvol to
    // 0 and stays running.
    let mut t10 = NOTE_ON_V1.to_vec();
    t10.extend(loadl(53, 11));
    t10.push(END);
    t10.extend(loadl(5, 1000));
    t10.extend([WAIT, 5]);
    let mut t11 = NOTE_ON_V1.to_vec();
    t11.extend(loadl(5, 1000));
    t11.extend([WAIT, 5]);
    let mut player = Player::new(vec![module(&[&t10, &t11], 1)], long_sample("one.xa"), 1);
    play(&mut player, "e1");
    play(&mut player, "e0");
    // Block 0: both tracks' notes play (sum = 20000).
    let b0 = player.render(BLOCK, &NoSources);
    assert_eq!(b0[0], 20_000);
    // Block 1: track 10 dies immediately (own FADE_ON == 0 && kill_after_fade),
    // while track 11 begins fading (extvol 1024 -> 924, well below 10000).
    let b1 = player.render(BLOCK, &NoSources);
    assert!(b1[0] > 0 && b1[0] < 10_000, "b1={}", b1[0]);
    // After 11 more blocks, track 11 has faded to 0 (silent) but is STILL
    // running (not killed, because kill_after_fade was set on track 10).
    let _ = player.render(11 * BLOCK, &NoSources);
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    assert!(
        !player.is_idle(),
        "track 11 should remain running at extvol 0"
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn end_after_note_off_kills_immediately_and_end_at_eof_faults() {
    // 1) `note_on v1; note_on v1; loadl v1, 999; note_off v1; end`
    //    followed by `loadl 110, 99; end`: a track holds one active
    //    voice (0x58015a), and `note_off` stops that voice regardless of
    //    its operand register's value (0x57e192..0x57e1a9 → 0x5801a0).
    //    With no note playing, `end` kills the track at once (0x57dd3e)
    //    and does not run on into `loadl 110, 99`.
    let mut stopped = NOTE_ON_V1.to_vec();
    stopped.extend(NOTE_ON_V1);
    stopped.extend(loadl(5, 999));
    stopped.extend([3, 5, END]); // note_off v1 (overwritten); end
    let mut after = loadl(110, 99);
    after.push(END);
    let asks: &[u8] = &[72, 6, 1, 110, END];
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(
        vec![module(&[&stopped, &after, asks], 1)],
        long_sample("one.xa"),
        1,
    );
    play(&mut player, "e0");
    player.render(BLOCK, &probe);
    assert!(player.is_idle());
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e2".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(8, 0)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // 2) `note_on; end` at EOF: the run-on past `end` (0x57d8b3) hits
    //    the end of the program buffer, reports a fault, and stops the
    //    thread and its note.
    let at_eof: &[u8] = &[2, 5, END];
    let mut eof_player = Player::new(vec![module(&[at_eof], 1)], long_sample("one.xa"), 1);
    play(&mut eof_player, "e0");
    assert!(eof_player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    assert!(eof_player.is_idle());
    assert_eq!(eof_player.faults.len(), 1, "{:?}", eof_player.faults);
    assert!(
        eof_player.faults[0].contains("address outside the program"),
        "{:?}",
        eof_player.faults
    );
}

#[test]
fn refused_getsrcdatafield_preserves_active_note_for_end_fade() {
    // A refused `getsrcdatafield` logs a diagnostic (0x57f33a → 0x57f3f9
    // → 0x57f408) without killing the track or cutting its voice, so a
    // following `end` still sees the playing note and fades it out.
    let mut r0 = NOTE_ON_V1.to_vec();
    r0.extend(loadl(6, 21)); // field 21 is refused by Snapshot
    r0.extend([72, 7, 1, 6]); // getsrcdatafield v3, arg1, v2
    r0.push(END);
    r0.extend(loadl(5, 1000));
    r0.extend([WAIT, 5]);
    let mut player = Player::new(vec![module(&[&r0], 1)], long_sample("one.xa"), 1);
    let lot = on_lot(&[(9, 400)]);
    play_on(&mut player, 9, &lot);
    let b0 = player.render(BLOCK, &lot);
    assert_eq!(b0[0], 10_000);
    assert_eq!(player.faults.len(), 1, "{:?}", player.faults);
    assert!(player.faults[0].contains("Illegal source parameter request"));
    let b1 = player.render(BLOCK, &lot);
    assert!(b1[0] > 0 && b1[0] < 10_000, "b1={}", b1[0]);
    player.render(10 * BLOCK, &lot);
    assert!(player.is_idle());
}

#[test]
fn pause_and_stop_object_leave_idle_kind_53_voice_playing() {
    // In 1.91, `pause` walks only the running-track list
    // (0x574869..0x5748c9) and hold checks `is_running` (0x57dc3d);
    // `fade_and_kill` also checks `is_running` (0x57dca8). Neither stops
    // or silences an idle kind-53 voice (`thread == 0`).
    let module = module_with(&[&[END]], 1, 10, "seq=53,10,0,6\n");
    let mut player = Player::new(vec![module], long_sample("two.xa"), 1);
    let lot = on_lot(&[(9, 400)]);
    play_on(&mut player, 9, &lot);
    player.render(BLOCK, &lot);
    assert!(player.is_idle());
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "seq".into(),
        },
        &lot,
    );
    assert_eq!(player.render(2, &lot), [10_000, 10_000]);
    player.handle(&SoundEvent::StopObject { object: 9 }, &lot);
    assert_eq!(player.render(2, &lot), [10_000, 10_000]);
    player.handle(&SoundEvent::Pause, &lot);
    assert_eq!(player.render(2, &lot), [10_000, 10_000]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn muted_play_trk_writes_source_through_the_started_track_s_target() {
    // `0x57f261` writes `SOURCE` (47) to the started track via vtable
    // +0x84 (through its `target`). On a muted start (0x57d9a5), the
    // started track keeps its existing `target`, so `SOURCE` lands in
    // that target track instead of locally.
    let mut t10 = loadl(53, 11);
    t10.push(SET_LOOP);
    t10.extend(loadl(5, 1000));
    t10.extend([WAIT, 5, LOOP]);
    let mut t11 = loadl(5, 20);
    t11.extend([WAIT, 5, 72, 6, 1, 47, END]);
    let mut t12 = loadl(47, 77);
    t12.extend(loadl(5, 10));
    t12.extend([PLAY_TRK, 5, END]);
    let (hit, addresses) = assemble(&[&t10, &t11, &t12]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,muted_trk,1024,0,0,1,0,0x100,1\n\
         11=0,dest_trk,1024,0,0,0,0,0x101,1\n\
         12=0,caller_trk,1024,0,0,0,0,0x102,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         e2=1,12\n",
        addresses[0], addresses[1], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], samples(), 1);
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e1".into(),
        },
        &probe,
    );
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e0".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    // Mute group 1 so track 10's start returns early at 0x57d9a5 keeping
    // target = 11, then track 12 runs `play_trk 10` with SOURCE = 77.
    player.set_group_volume(1, 0);
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e2".into(),
        },
        &probe,
    );
    player.render(12 * BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(8, 77)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn stopped_target_persists_registers_and_clears_fade_on() {
    // Matches `SetMainMusicSongAndHitList`: targets stopped track 11,
    // writes `v1 = 11` and `FADE_ON = 1` (0x57e059 forces FADE_ON back
    // to 0 on a stopped track), reads both back, then runs `play_trk v1`.
    // Track 11 starts cleanly and still sees `v1 == 11` (outside BANKS,
    // 0x57da3a..0x57da87).
    let mut t10 = loadl(53, 11);
    t10.extend(loadl(5, 11));
    t10.extend(loadl(45, 1));
    t10.extend([72, 6, 1, 45]); // query FADE_ON on stopped target -> 0
    t10.extend([72, 6, 1, 5]); // query v1 on stopped target -> 11
    t10.extend([PLAY_TRK, 5, END]);
    let t11: &[u8] = &[72, 6, 1, 5, END]; // query own v1 -> 11
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(vec![module(&[&t10, t11], 1)], samples(), 1);
    play_on(&mut player, 8, &probe.lot);
    player.render(4 * BLOCK, &probe);
    // Both reads in track 10 go through `target = 11` (so stopped track 11's
    // `arg1` is 0); track 11's own read receives `arg1 = 8` from `play_trk`.
    assert_eq!(*probe.asked.borrow(), [(0, 0), (0, 11), (8, 11)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn routed_volume_write_to_idle_kind_53_voice_updates_level_immediately() {
    // Track 10 ends immediately and kind 53 starts an idle voice on it.
    // Track 11 targets stopped track 10 and writes EXTVOL = 0, then
    // waits and restores EXTVOL = 1024. The idle voice updates at once
    // (0x57df13 -> 0x57f759).
    let mut t11 = loadl(53, 10);
    t11.extend(loadl(19, 0));
    t11.extend(loadl(5, 20));
    t11.extend([WAIT, 5]);
    t11.extend(loadl(19, 1024));
    t11.push(END);
    let (hit, addresses) = assemble(&[&[END], &t11]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         10=0,idle_trk,1024,0,0,0,0,0x100,1\n\
         11=0,writer_trk,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         seq=53,10,0,6\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("two.xa"), 1);
    let lot = on_lot(&[(9, 400)]);
    play_on(&mut player, 9, &lot);
    player.render(BLOCK, &lot);
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "seq".into(),
        },
        &lot,
    );
    assert_eq!(player.render(BLOCK, &lot)[0], 10_000);

    // Track 11 silences the idle voice on block 0 of its run.
    play(&mut player, "e1");
    let silenced = player.render(BLOCK, &lot);
    assert!(silenced.iter().all(|&s| s == 0), "{silenced:?}");
    // After the 20 ms wait, track 11 restores EXTVOL = 1024.
    let _ = player.render(10 * BLOCK, &lot);
    assert_eq!(player.render(BLOCK, &lot)[0], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn muted_play_trk_writes_source_to_stopped_track_and_spares_idle_voice() {
    // `0x57f261` writes `SOURCE` (47) to a muted, stopped destination
    // (including one with an idle kind-53 voice) without changing that
    // idle voice's gain when its group is muted.
    let mut t11 = loadl(47, 77);
    t11.extend(loadl(5, 10));
    t11.extend([PLAY_TRK, 5]);
    t11.extend(loadl(53, 10));
    t11.extend([72, 6, 1, 47, END]);
    let (hit, addresses) = assemble(&[&[END], &t11]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         10=0,muted_stopped,1024,0,0,1,0,0x100,1\n\
         11=0,caller,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         seq=53,10,0,6\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("two.xa"), 1);
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    // Start an idle kind-53 voice on stopped track 10 while group 1 is unmuted,
    // then mute group 1 before track 11 runs `play_trk 10`.
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "seq".into(),
        },
        &probe,
    );
    assert_eq!(player.render(BLOCK, &probe)[0], 10_000);
    player.set_group_volume(1, 0);
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e1".into(),
        },
        &probe,
    );
    let out = player.render(2 * BLOCK, &probe);
    assert_eq!(out[out.len() - 1], 10_000);
    // Both `arg1` (0) and `SOURCE` (77) are read through `target = 10`.
    assert_eq!(*probe.asked.borrow(), [(0, 77)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn note_on_preserves_its_operand_register_and_flags() {
    // `0x57e17f -> 0x57f595` passes the raw operand byte without writing
    // back a note ID or changing comparison flags (relied on by install
    // scripts such as `simsgeneratedhitsource.hit @0x18b1`).
    const IFLT: u8 = 65;
    let mut r0 = loadl(5, 42);
    r0.extend(loadl(6, -5)); // comparison flags < 0
    r0.extend(NOTE_ON_V1); // must preserve v1 == 42 and flags < 0
    r0.push(IFLT);
    r0.extend(long(0x101));
    r0.extend([3, 5, END]); // unreachable if flags stayed < 0
    let mut r1 = vec![72, 6, 1, 5]; // query v1 -> 42
    r1.extend([3, 5, END]); // note_off; end
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(vec![module(&[&r0, &r1], 1)], long_sample("one.xa"), 1);
    play_on(&mut player, 8, &probe.lot);
    player.render(2 * BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(8, 42)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kill_paths_preserve_locals_clear_patch_and_kind_53_persists_patch() {
    // 1) StopObject on a silent track, immediate `seqgroup_kill` (extvol == 0),
    //    and StopAll all save the stopped register bank (preserving `v1 = 77`
    //    and clearing `PATCH = 0` per 0x57dd62 -> 0x57f682).
    // 2) Kind 53 writes `PATCH` to the stopped track object (0x57e0cc), so it
    //    remains readable after the short idle sample finishes.
    let mut writer = loadl(5, 77);
    writer.extend(loadl(19, 0)); // extvol = 0 for immediate seqgroup_kill
    writer.extend(loadl(6, 1));
    writer.extend([SEQGROUP_KILL, 6]); // claim seqgroup 1
    writer.push(SET_LOOP);
    writer.extend(loadl(7, 1000));
    writer.extend([WAIT, 7, LOOP]);

    let mut killer = loadl(6, 1);
    killer.extend([SEQGROUP_KILL, 6, END]);

    let mut reader = loadl(53, 10);
    reader.extend([GETSRCDATAFIELD, 6, 1, 5]); // read track 10's v1
    reader.extend([GETSRCDATAFIELD, 6, 1, 50]); // read track 10's PATCH
    reader.push(END);

    let (hit, addresses) = assemble(&[&writer, &killer, &reader]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         10=0,victim,1024,2,0,0,0,0x100,1\n\
         11=0,killer,1024,0,0,0,0,0x101,1\n\
         12=0,reader,1024,0,0,0,0,0x102,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         e2=1,12\n\
         seq=53,10,0,6\n",
        addresses[0], addresses[1], addresses[2]
    );
    let make_player = || {
        let m = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
        Player::new(vec![m], samples(), 1)
    };

    // Case A: StopObject on silent running track
    let probe_a = Probe {
        lot: on_lot(&[(9, 400)]),
        asked: Default::default(),
    };
    let mut pa = make_player();
    play_on(&mut pa, 9, &probe_a.lot);
    pa.render(BLOCK, &probe_a);
    pa.handle(&SoundEvent::StopObject { object: 9 }, &probe_a);
    pa.handle(
        &SoundEvent::Play {
            object: 9,
            name: "e2".into(),
        },
        &probe_a,
    );
    pa.render(2 * BLOCK, &probe_a);
    assert_eq!(*probe_a.asked.borrow(), [(9, 77), (9, 0)]);
    assert!(pa.faults.is_empty(), "{:?}", pa.faults);

    // Case B: immediate `seqgroup_kill` (extvol == 0)
    let probe_b = Probe {
        lot: on_lot(&[(9, 400)]),
        asked: Default::default(),
    };
    let mut pb = make_player();
    play_on(&mut pb, 9, &probe_b.lot);
    pb.render(BLOCK, &probe_b);
    pb.handle(
        &SoundEvent::Play {
            object: 9,
            name: "e1".into(),
        },
        &probe_b,
    );
    pb.render(BLOCK, &probe_b);
    pb.handle(
        &SoundEvent::Play {
            object: 9,
            name: "e2".into(),
        },
        &probe_b,
    );
    pb.render(2 * BLOCK, &probe_b);
    assert_eq!(*probe_b.asked.borrow(), [(9, 77), (9, 0)]);
    assert!(pb.faults.is_empty(), "{:?}", pb.faults);

    // Case C: StopAll preserves both running-track and already-saved banks
    let probe_c = Probe {
        lot: on_lot(&[(9, 400)]),
        asked: Default::default(),
    };
    let mut pc = make_player();
    play_on(&mut pc, 9, &probe_c.lot);
    pc.render(BLOCK, &probe_c);
    pc.handle(&SoundEvent::StopAll, &probe_c);
    pc.handle(&SoundEvent::StopAll, &probe_c); // second StopAll keeps saved bank
    pc.handle(
        &SoundEvent::Play {
            object: 9,
            name: "e2".into(),
        },
        &probe_c,
    );
    pc.render(2 * BLOCK, &probe_c);
    assert_eq!(*probe_c.asked.borrow(), [(9, 77), (9, 0)]);

    // Case D: kind 53 writes PATCH=2 to the stopped bank and keeps it after playback ends
    pc.handle(
        &SoundEvent::Play {
            object: 9,
            name: "seq".into(),
        },
        &probe_c,
    );
    pc.render_until_idle(RATE as usize, &probe_c);
    assert!(pc.is_idle());
    probe_c.asked.borrow_mut().clear();
    pc.handle(
        &SoundEvent::Play {
            object: 9,
            name: "e2".into(),
        },
        &probe_c,
    );
    pc.render(2 * BLOCK, &probe_c);
    assert_eq!(*probe_c.asked.borrow(), [(9, 77), (9, 2)]);
    assert!(pc.faults.is_empty(), "{:?}", pc.faults);
}

#[test]
fn patch_write_stops_idle_kind_53_voice_and_is3d_does_not_refresh_muted_idle_voice() {
    // 1) Writing `Is3d` (26) to a stopped track with an idle kind-53 voice
    //    after muting its group must NOT refresh/silence the voice (`0x57df08`
    //    refreshes on `IsHeadRelative` = 27, not `Is3d` = 26).
    // 2) Writing `PATCH = 0` (`0x32` -> `0x57f625`) stops the idle voice at once.
    let mut write_is3d = loadl(53, 10);
    write_is3d.extend(loadl(26, 0));
    write_is3d.push(END);
    let mut clear_patch = loadl(53, 10);
    clear_patch.extend(loadl(50, 0));
    clear_patch.push(END);
    let (hit, addresses) = assemble(&[&[END], &write_is3d, &clear_patch]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         10=0,idle_trk,1024,0,0,1,0,0x100,1\n\
         11=0,is3d_writer,1024,0,0,0,0,0x101,1\n\
         12=0,patch_writer,1024,0,0,0,0,0x102,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         e2=1,12\n\
         seq=53,10,0,6\n",
        addresses[0], addresses[1], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("two.xa"), 1);
    let lot = on_lot(&[(9, 400)]);
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "seq".into(),
        },
        &lot,
    );
    assert_eq!(player.render(BLOCK, &lot)[0], 10_000);
    player.set_group_volume(1, 0);
    // Writing Is3d (26) leaves the idle voice audible.
    play(&mut player, "e1");
    assert_eq!(player.render(2 * BLOCK, &lot)[BLOCK], 10_000);
    // Writing PATCH = 0 tears down the idle voice immediately.
    play(&mut player, "e2");
    let after = player.render(BLOCK, &lot);
    assert!(after.iter().all(|&s| s == 0), "{after:?}");
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn camera_places_silent_instance_and_killed_hold_does_not_gate_idle_voice() {
    // 1) Moving the camera while a track with a recorded instance is silent
    //    writes placement to its persistent stopped bank (`0x574728`,
    //    `0x574732`), so a following kind-53 voice starts panned right.
    // 2) Pausing a running track, killing it with `kill_trk`, and then
    //    starting a kind-53 voice plays audibly (hold cleared on kill and
    //    idle voices never gated by `held`).
    let mut hold_me = loadl(5, 1000);
    hold_me.extend([WAIT, 5, END]);
    let mut kill_10 = loadl(5, 10);
    kill_10.extend([KILL_TRK, 5, END]);
    let (hit, addresses) = assemble(&[&[END], &hold_me, &kill_10]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         10=0,silent_trk,1024,2,0,0,0,0x100,1\n\
         11=0,held_trk,1024,0,0,0,0,0x101,1\n\
         12=0,killer_trk,1024,0,0,0,0,0x102,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         e2=1,12\n\
         seq10=53,10,0,6\n\
         seq11=53,11,0,6\n",
        addresses[0], addresses[1], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("two.xa"), 1);
    let center = on_lot(&[(9, 400)]);
    let right = on_lot(&[(9, 800)]);

    // Part 1: track 10 records object 9 centered, ends immediately, camera moves right.
    play_on(&mut player, 9, &center);
    player.render(BLOCK, &center);
    assert!(player.is_idle());
    player.camera_moved(0, u32::MAX, &right);
    player.handle(
        &SoundEvent::Play {
            object: 9,
            name: "seq10".into(),
        },
        &right,
    );
    let stereo = player.render_stereo(BLOCK, &right);
    let [left_s, right_s] = [stereo[0], stereo[1]];
    assert!(
        right_s > left_s && left_s > 0,
        "left={left_s} right={right_s}"
    );

    // Part 2: pause track 11, kill it via `kill_trk`, then play kind 53 on it.
    let mut kill_11 = loadl(5, 11);
    kill_11.extend([KILL_TRK, 5, END]);
    let (hit2, a2) = assemble(&[&hold_me, &kill_11]);
    let hot2 = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         11=0,held_trk,1024,0,0,0,0,0x100,1\n\
         12=0,killer_trk,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         e0=1,11\n\
         e1=1,12\n\
         seq11=53,11,0,6\n",
        a2[0], a2[1]
    );
    let m2 = Module::parse("test", &hit2, b"v1 5\n", hot2.as_bytes()).unwrap();
    let mut p2 = Player::new(vec![m2], long_sample("two.xa"), 1);
    play(&mut p2, "e0");
    p2.render(BLOCK, &NoSources);
    p2.handle(&SoundEvent::Pause, &NoSources);
    play(&mut p2, "e1"); // kill_trk 11 while track 11 is held
    p2.render(BLOCK, &NoSources);
    p2.handle(
        &SoundEvent::Play {
            object: 9,
            name: "seq11".into(),
        },
        &center,
    );
    assert_eq!(p2.render(BLOCK, &center)[0], 10_000);
    assert!(p2.faults.is_empty(), "{:?}", p2.faults);
}

#[test]
fn routed_main_duckpri_on_stopped_track_updates_global() {
    // `0x57e011 -> 0x57d2cb` updates the target track's `main_duckpri`
    // request even when the target track is stopped.
    let mut writer = loadl(53, 11);
    writer.extend(loadl(123, 100));
    writer.push(END);
    let reader: &[u8] = &[GETSRCDATAFIELD, 6, 1, 123, END];
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(vec![module(&[&writer, &[END], reader], 1)], samples(), 1);
    play_on(&mut player, 8, &probe.lot);
    player.render(BLOCK, &probe);
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e2".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(8, 100)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn patch_write_on_running_track_tears_down_note_so_end_kills_without_run_on() {
    // `0x57dfe8 -> 0x57e040 -> 0x57f625`: writing `PATCH` on a running
    // track immediately tears down its active voice, so a following `end`
    // takes the immediate-kill branch and never runs trailing instructions.
    let mut r0 = NOTE_ON_V1.to_vec();
    r0.extend(loadl(50, 0)); // PATCH = 0 stops the playing voice
    r0.push(END);
    r0.extend(loadl(110, 99)); // must NOT run
    r0.push(END);
    let ask: &[u8] = &[GETSRCDATAFIELD, 6, 1, 110, END];
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(vec![module(&[&r0, ask], 1)], long_sample("one.xa"), 1);
    play_on(&mut player, 8, &probe.lot);
    let out = player.render(BLOCK, &probe);
    assert!(out.iter().all(|&s| s == 0), "{out:?}");
    assert!(player.is_idle());
    player.handle(
        &SoundEvent::Play {
            object: 8,
            name: "e1".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(8, 0)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn killing_a_stopped_track_clears_patch_and_main_duckpri() {
    // `kill_trk`, `StopAll`, and zero-`EXTVOL` `seqgroup_kill` all call
    // `0x57dd3e` unconditionally on a stopped track, clearing both `PATCH`
    // (written by kind 53) and `MAIN_DUCKPRI` (written through `target`).
    let mut claim_and_stop = loadl(5, 1);
    claim_and_stop.extend([SEQGROUP_RETURN, 5]);
    claim_and_stop.extend(loadl(19, 0)); // extvol = 0
    claim_and_stop.push(END);

    let mut set_duck = loadl(53, 10);
    set_duck.extend(loadl(123, 100));
    set_duck.push(END);

    let mut kill_10 = loadl(5, 10);
    kill_10.extend([KILL_TRK, 5, END]);

    let mut seq_kill = loadl(5, 1);
    seq_kill.extend([SEQGROUP_KILL, 5, END]);

    let mut read_state = loadl(53, 10);
    read_state.extend([GETSRCDATAFIELD, 6, 1, 50]); // PATCH on stopped track 10
    read_state.extend([GETSRCDATAFIELD, 6, 1, 123]); // global MAIN_DUCKPRI
    read_state.push(END);

    let (hit, addresses) =
        assemble(&[&claim_and_stop, &set_duck, &kill_10, &seq_kill, &read_state]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         0x103={:#x}\n\
         0x104={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         2=0,two,\"two.xa\"\n\
         [GlobalHitList]\n\
         6=2\n\
         [Track]\n\
         10=0,victim,1024,0,0,0,0,0x100,1\n\
         11=0,ducker,1024,0,0,0,0,0x101,1\n\
         12=0,killtrk,1024,0,0,0,0,0x102,1\n\
         13=0,seqkill,1024,0,0,0,0,0x103,1\n\
         14=0,reader,1024,0,0,0,0,0x104,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         e2=1,12\n\
         e3=1,13\n\
         e4=1,14\n\
         seq=53,10,0,6\n",
        addresses[0], addresses[1], addresses[2], addresses[3], addresses[4]
    );
    for mode in ["kill_trk", "stop_all", "seqgroup_kill"] {
        let m = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
        let mut p = Player::new(vec![m], samples(), 1);
        let probe = Probe {
            lot: on_lot(&[(8, 400)]),
            asked: Default::default(),
        };
        play(&mut p, "e0"); // track 10 claims group 1, sets extvol = 0, ends
        p.render(BLOCK, &probe);
        // Set PATCH = 2 via kind 53 (let short sample finish) and MAIN_DUCKPRI = 100.
        play(&mut p, "seq");
        p.render_until_idle(RATE as usize, &probe);
        play(&mut p, "e1");
        p.render(BLOCK, &probe);
        match mode {
            "kill_trk" => {
                play(&mut p, "e2");
                p.render(BLOCK, &probe);
            }
            "stop_all" => {
                p.handle(&SoundEvent::StopAll, &probe);
            }
            "seqgroup_kill" => {
                play(&mut p, "e3");
                p.render(BLOCK, &probe);
            }
            _ => unreachable!(),
        }
        play(&mut p, "e4");
        p.render(BLOCK, &probe);
        assert_eq!(*probe.asked.borrow(), [(9, 0), (9, 0)], "mode={mode}");
        assert!(p.faults.is_empty(), "mode={mode}: {:?}", p.faults);
    }
}

#[test]
fn seqgroup_kill_routes_holder_extvol_and_fade_writes_through_target() {
    // `0x57ef5b` reads `EXTVOL` through the holder's `target`: track 10
    // has local `EXTVOL = 1024`, plays a note, claims seqgroup 1, and
    // targets stopped track 11 whose `EXTVOL` is 0. When track 12 runs
    // `seqgroup_kill 1`, it reads 0 through track 10's target and kills
    // track 10 immediately.
    let mut set_t11_zero = loadl(53, 11);
    set_t11_zero.extend(loadl(19, 0));
    set_t11_zero.push(END);

    let mut holder = NOTE_ON_V1.to_vec();
    holder.extend(loadl(5, 1));
    holder.extend([SEQGROUP_RETURN, 5]);
    holder.extend(loadl(53, 11)); // target stopped track 11 (extvol = 0)
    holder.push(SET_LOOP);
    holder.extend(loadl(6, 1000));
    holder.extend([WAIT, 6, LOOP]);

    let mut killer = loadl(5, 1);
    killer.extend([SEQGROUP_KILL, 5, END]);

    let mut player = Player::new(
        vec![module(&[&holder, &[END], &killer, &set_t11_zero], 1)],
        long_sample("one.xa"),
        1,
    );
    play(&mut player, "e3"); // write extvol = 0 on stopped track 11
    player.render(BLOCK, &NoSources);
    play(&mut player, "e0"); // holder runs with local extvol 1024, target 11
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    play(&mut player, "e2"); // seqgroup_kill 1 kills holder immediately
    let after = player.render(BLOCK, &NoSources);
    assert!(after.iter().all(|&s| s == 0), "{after:?}");
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kind_18_writes_fade_registers_and_clears_fade_on_for_stopped_track() {
    // `0x5739d4..0x5739fe` writes `FADE_DEST = 0`, `FADE_VAR = EXTVOL (19)`,
    // `FADE_SPEED = 75`, and `FADE_ON = 1` (forced to 0 by `0x57e059`) on a
    // stopped registered track.
    let mut reader = loadl(53, 10);
    reader.extend([GETSRCDATAFIELD, 6, 1, 40]); // FADE_DEST
    reader.extend([GETSRCDATAFIELD, 6, 1, 41]); // FADE_VAR
    reader.extend([GETSRCDATAFIELD, 6, 1, 42]); // FADE_SPEED
    reader.extend([GETSRCDATAFIELD, 6, 1, 45]); // FADE_ON
    reader.push(END);
    let module = module_with(&[&[END], &reader], 1, 10, "fade75=18,10,75,0,1\n");
    let mut player = Player::new(vec![module], samples(), 1);
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    play(&mut player, "fade75");
    play(&mut player, "e1");
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(0, 0), (0, 19), (0, 75), (0, 0)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn never_started_track_reads_zero_patch_after_kill_trk_and_stop_all() {
    // In 1.91 (`0x57db54..0x57db67`), the HOT patch ID is loaded into live
    // register `PATCH` (50) only on a fresh start; a never-started track
    // reads `PATCH == 0`, including after `kill_trk` or `StopAll`.
    let mut kill_10 = loadl(5, 10);
    kill_10.extend([KILL_TRK, 5, END]);
    let mut read_patch = loadl(53, 10);
    read_patch.extend([GETSRCDATAFIELD, 6, 1, 50, END]);
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    let mut player = Player::new(
        vec![module(&[&[END], &kill_10, &read_patch], 1)],
        samples(),
        1,
    );
    play(&mut player, "e1"); // kill_trk 10 (never started)
    player.render(BLOCK, &probe);
    play(&mut player, "e2"); // read track 10's PATCH -> 0
    player.render(BLOCK, &probe);
    player.handle(&SoundEvent::StopAll, &probe);
    play(&mut player, "e2"); // read track 10's PATCH -> 0
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(0, 0), (0, 0)]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kinds_37_38_39_map_slider_steps_to_groups_and_ambience() {
    use sims_audio::fsc::Score;
    use sims_audio::levels::{Inputs, levels};
    use sims_audio::player::VOLUME_STEPS;

    // Three tracks in groups 1 (Sfx), 2 (Music) and 3 (Vox), each looping a
    // constant sample of 4000, plus a 1-cell looping ambience score to verify
    // kind 37 updates ambience volume (0x5740b4..0x5740f1) while kinds 38
    // and 39 leave it alone.
    let loop_note: &[u8] = &[SET_LOOP, NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, LOOP];
    let (hit, addresses) = assemble(&[loop_note]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,sfx,1024,0,0,1,0,0x100,1\n\
         11=0,mus,1024,0,0,2,0,0x100,1\n\
         12=0,vox,1024,0,0,3,0,0x100,1\n\
         [EventMapping]\n\
         sfx=1,10\n\
         mus=1,11\n\
         vox=1,12\n\
         amb0=52,0\n\
         fx0=37,0\n\
         fx5=37,5\n\
         fx_bad=37,11\n\
         fx_neg=37,-1\n\
         mus3=38,3\n\
         vox7=39,7\n\
         mus_bad=38,12\n\
         vox_bad=39,-2\n",
        addresses[0]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let score = Score::parse(
        b"version 7\n#\tvol\tpri\n\
          \t1024\t0\t1\t1\t1\t1\t600\t2\t1\t1\t0\t0\t0\t0\n\
          #\tvol\n\
          \t1024\t0\t512\t512\t0\t0\t0\t0\t0\t0\t1\t0\t20\t0\t0\t0\t0\t0\tamb.xa\n",
    )
    .unwrap();
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    map.insert(
        "dir\\amb.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    let mut player = Player::new(vec![module], map, 1);
    assert!(player.add_score(0, &score, "dir"));

    // Sound manager constructor (0x5713ee..0x571409) starts all three
    // sliders at step 10 (1024).
    assert_eq!(
        (
            player.fx_volume_step(),
            player.music_volume_step(),
            player.vox_volume_step()
        ),
        (10, 10, 10)
    );
    assert_eq!(
        VOLUME_STEPS,
        [0, 300, 390, 490, 590, 690, 790, 840, 895, 960, 1024]
    );

    let expected_mono = |group_volume: i32| -> i16 {
        let g = levels(&Inputs {
            vol: 1024,
            extvol: 1024,
            pan: 512,
            pitch: 3600,
            duck_priority: 0,
            three_d: false,
            group_volume,
            main_duck_priority: 0,
            master: 1024,
        })
        .gains;
        let l = (4000i64 * i64::from(g[0])) >> 16;
        let r = (4000i64 * i64::from(g[1])) >> 16;
        ((l + r) / 2) as i16
    };

    // Kind 37 (step 5 -> 690), kind 38 (step 3 -> 490), kind 39 (step 7 -> 840).
    for (event_name, track_event, step) in
        [("fx5", "sfx", 5), ("mus3", "mus", 3), ("vox7", "vox", 7)]
    {
        player.handle(&SoundEvent::StopAll, &NoSources);
        play(&mut player, event_name);
        play(&mut player, track_event);
        let out = player.render(BLOCK, &NoSources);
        assert_eq!(
            out[0],
            expected_mono(VOLUME_STEPS[step]),
            "{event_name} (step {step})"
        );
    }
    assert_eq!(
        (
            player.fx_volume_step(),
            player.music_volume_step(),
            player.vox_volume_step()
        ),
        (5, 3, 7)
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // Kind 37 updates ambience score volume (0x5740b4..0x5740f1), while
    // kinds 38 and 39 leave ambience volume unchanged.
    player.handle(&SoundEvent::StopAll, &NoSources);
    play(&mut player, "amb0");
    let amb_at_5 = player.render(RATE as usize / 2, &NoSources);
    let sample_at_5 = *amb_at_5.last().unwrap();
    assert_eq!(sample_at_5, expected_mono(VOLUME_STEPS[5]));
    play(&mut player, "mus3");
    play(&mut player, "vox7");
    assert_eq!(player.render(BLOCK, &NoSources)[0], sample_at_5);
    play(&mut player, "fx0");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 0);
    player.handle(&SoundEvent::StopAll, &NoSources);
    play(&mut player, "fx5");

    // Out-of-range HOT event steps report faults without changing state.
    for bad in ["fx_bad", "fx_neg", "mus_bad", "vox_bad"] {
        play(&mut player, bad);
    }
    assert_eq!(player.faults.len(), 4, "{:?}", player.faults);
    assert_eq!(
        (
            player.fx_volume_step(),
            player.music_volume_step(),
            player.vox_volume_step()
        ),
        (5, 3, 7)
    );

    // Direct host methods clamp steps above 10 to 10.
    player.set_fx_volume_step(99, &NoSources);
    player.set_music_volume_step(99);
    player.set_vox_volume_step(99);
    assert_eq!(
        (
            player.fx_volume_step(),
            player.music_volume_step(),
            player.vox_volume_step()
        ),
        (10, 10, 10)
    );
}

#[test]
fn kind_37_reruns_day_night_loop_and_restarts_after_mute() {
    use sims_audio::levels::{Inputs, levels};
    use sims_audio::player::{NITE_LOOP, VOLUME_STEPS};

    let loop_note: &[u8] = &[SET_LOOP, NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, LOOP];
    // Reads EXTVOL (19), FADE_DEST (40), FADE_VAR (41), FADE_SPEED (42), FADE_ON (45)
    // from NITE_LOOP through `target`.
    let mut inspect = loadl(53, NITE_LOOP as i32);
    for reg in [19u8, 40, 41, 42, 45] {
        inspect.extend([GETSRCDATAFIELD, 6, 1, reg]);
    }
    inspect.push(END);
    let (hit, addresses) = assemble(&[loop_note, &inspect]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         {NITE_LOOP}=0,nite_loop,1024,0,0,1,0,0x100,1\n\
         20=0,inspect,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         m3=36,3\n\
         m0=36,0\n\
         fx0=37,0\n\
         fx5=37,5\n\
         fx8=37,8\n\
         mus5=38,5\n\
         vox5=39,5\n\
         inspect=1,20\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![4000; RATE as usize],
        },
    );
    let mut player = Player::new(vec![module], map, 1);

    let snap = |hour: i32| {
        let mut s = Snapshot::default();
        s.absent[4] = hour;
        s.lot = 1;
        s
    };
    let night = snap(22);
    let day = snap(12);

    // Mute effects before entering live mode at night: `0x574978` picks
    // NITE_LOOP, `start` returns early on group 1 volume 0, and
    // `0x574a1e..0x574a5e` still writes the stopped fade setup (FADE_ON=0).
    for name in ["fx0", "m3", "m0"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &night,
        );
    }
    // `m3`/`m0` report missing music tracks (517, 2957, 519) in this
    // minimal module; clear those so later assertions catch any new faults.
    player.faults.clear();
    assert!(
        player
            .render_stereo(BLOCK * 4, &night)
            .iter()
            .all(|&s| s == 0)
    );
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "inspect".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    assert_eq!(
        *probe.asked.borrow(),
        [(0, 0), (0, 0), (0, 19), (0, 5000), (0, 0)]
    );

    // Kinds 38 and 39 do NOT run 0x574978.
    for name in ["mus5", "vox5"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &night,
        );
    }
    assert!(
        player
            .render_stereo(BLOCK * 4, &night)
            .iter()
            .all(|&s| s == 0)
    );

    // Raising the effects slider via kind 37 (step 5 -> 690) with NO
    // `camera_moved` runs 0x574978, starts `nite_loop`, and fades `extvol`
    // up to 690 (at group 1 volume 690).
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "fx5".into(),
        },
        &night,
    );
    let out = player.render_stereo(BLOCK * 4, &night);
    let expected = levels(&Inputs {
        vol: 1024,
        extvol: VOLUME_STEPS[5],
        pan: 512,
        pitch: 3600,
        duck_priority: 0,
        three_d: false,
        group_volume: VOLUME_STEPS[5],
        main_duck_priority: 0,
        master: 1024,
    })
    .gains;
    let want = ((4000i64 * i64::from(expected[0])) >> 16) as i16;
    assert_eq!(out[out.len() - 2], want, "nite_loop at fx step 5");

    // When the hour is daytime, kind 37 runs 0x574978 and fades `nite_loop`
    // out even without a camera move.
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "fx8".into(),
        },
        &day,
    );
    let _ = player.render_stereo(BLOCK * 16, &day);
    assert!(
        player
            .render_stereo(BLOCK * 4, &day)
            .iter()
            .all(|&s| s == 0),
        "kind 37 in daytime should stop nite_loop"
    );

    // Inactive sound: kind 37 still updates slider step / group volume, and
    // 0x574978 takes the stop branch instead of starting `nite_loop`.
    player.set_active(false, &night);
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "fx5".into(),
        },
        &night,
    );
    assert_eq!(player.fx_volume_step(), 5);
    assert!(
        player
            .render_stereo(BLOCK * 4, &night)
            .iter()
            .all(|&s| s == 0)
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn day_night_fade_target_reads_fx_slider_step_not_raw_group_volume() {
    use sims_audio::player::NITE_LOOP;

    // `0x574a07..0x574a10` reads `VOLUME_STEPS[+0xf0]` independently of raw
    // `group_volumes[1]` (`0x57c102`). With fx slider at default step 10
    // (1024) and raw `set_group_volume(1, 0)`, a muted start still writes
    // `FADE_DEST = 1024`, and with `set_group_volume(1, 512)`, `nite_loop`
    // starts with `FADE_DEST = 1024` (not 512).
    let loop_note: &[u8] = &[SET_LOOP, NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, LOOP];
    let mut inspect = loadl(53, NITE_LOOP as i32);
    inspect.extend([GETSRCDATAFIELD, 6, 1, 40, END]); // FADE_DEST
    let (hit, addresses) = assemble(&[loop_note, &inspect]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         {NITE_LOOP}=0,nite_loop,1024,0,0,1,0,0x100,1\n\
         20=0,inspect,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         m3=36,3\n\
         m0=36,0\n\
         inspect=1,20\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("one.xa"), 1);
    let mut night = Snapshot::default();
    night.absent[4] = 22;
    night.lot = 1;

    // Slider stays at 10 (1024), raw group 1 volume set to 0 -> stopped
    // track still receives FADE_DEST = 1024.
    player.set_group_volume(1, 0);
    for name in ["m3", "m0"] {
        player.handle(
            &SoundEvent::Play {
                object: 0,
                name: name.into(),
            },
            &night,
        );
    }
    let probe = Probe {
        lot: on_lot(&[(8, 400)]),
        asked: Default::default(),
    };
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "inspect".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(0, 1024)]);

    // Raw group 1 volume set to 512 -> nite_loop starts with FADE_DEST = 1024.
    player.set_group_volume(1, 512);
    player.camera_moved(0, 22, &night);
    probe.asked.borrow_mut().clear();
    player.handle(
        &SoundEvent::Play {
            object: 0,
            name: "inspect".into(),
        },
        &probe,
    );
    player.render(BLOCK, &probe);
    assert_eq!(*probe.asked.borrow(), [(0, 1024)]);
}

#[test]
fn group_volume_refresh_reads_voice_levels_through_target() {
    // `0x57c197` -> `0x57f759` reads `VOL`, `EXTVOL`, `DUCKPRI`, and `Is3d`
    // through `target` (`vtable[+0x88]` -> `0x57de16`), while keeping `PAN`
    // local (`0x57f85f`). Track 10 (group 1) starts a note at local
    // EXTVOL=1024, then targets stopped track 11 whose EXTVOL was set to 0.
    // A kind 37 slider update refreshes track 10's voice through `0x57f759`,
    // silencing it immediately and keeping it silent on later blocks.
    let mut set_t11_zero = loadl(53, 11);
    set_t11_zero.extend(loadl(19, 0));
    set_t11_zero.push(END);

    let mut targeter = NOTE_ON_V1.to_vec();
    targeter.extend(loadl(53, 11));
    targeter.extend([SET_LOOP, LOOP]);

    let (hit, addresses) = assemble(&[&targeter, &[END], &set_t11_zero]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,t10,1024,0,0,1,0,0x100,1\n\
         11=0,t11,1024,0,0,1,0,0x101,1\n\
         12=0,t12,1024,0,0,0,0,0x102,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e2=1,12\n\
         fx8=37,8\n",
        addresses[0], addresses[1], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("one.xa"), 1);

    play(&mut player, "e2"); // write EXTVOL = 0 on stopped track 11
    player.render(BLOCK, &NoSources);
    play(&mut player, "e0"); // starts note at local EXTVOL=1024, then sets target=11
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);

    // Slider change refreshes group 1 tracks via 0x57f759 -> reads EXTVOL=0
    // through target 11 -> silent immediately and on subsequent blocks.
    play(&mut player, "fx8");
    assert!(player.render(BLOCK * 3, &NoSources).iter().all(|&s| s == 0));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn slider_and_ducking_updates_leave_held_voices_latched() {
    // `0x57dc5a` removes held tracks from the running list walked by both
    // `0x57c0ea` (group volume) and `0x57cc74` (`main_duckpri`). A held note
    // therefore keeps its latched gain across slider/ducking updates until
    // something refreshes it after resuming.
    let loop_note: &[u8] = &[SET_LOOP, NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, LOOP];
    let mut set_duck = loadl(123, 5);
    set_duck.extend([SET_LOOP, LOOP]);
    let (hit, addresses) = assemble(&[loop_note, &set_duck]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,sfx,1024,0,0,1,0,0x100,1\n\
         11=0,ducker,1024,0,0,0,0,0x101,1\n\
         [EventMapping]\n\
         sfx=1,10\n\
         ducker=1,11\n\
         fx0=37,0\n",
        addresses[0], addresses[1]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("one.xa"), 1);

    play(&mut player, "sfx");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    player.handle(&SoundEvent::Pause, &NoSources);
    // Lower FX slider to 0 and have an unheld track raise main_duckpri
    // while track 10 is held.
    play(&mut player, "fx0");
    play(&mut player, "ducker");
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    player.handle(&SoundEvent::Unpause, &NoSources);
    // Track 10's resumed note still plays at its latched gain (10_000)
    // because 0x57c0ea and 0x57cc74 skipped it while held.
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn duckpri_and_is3d_latch_until_main_duckpri_refresh_including_unchanged_max_and_stopped_target() {
    // 1) Writing `DUCKPRI` (25) or `Is3d` (26) does NOT call 0x57f759
    //    (0x57def1..0x57e057), so a ducked note stays ducked.
    // 2) A subsequent `MAIN_DUCKPRI` write refreshes all unheld running
    //    voices at 0x57d31e -> 0x57cc74 EVEN when the maximum stays the same.
    // 3) A `MAIN_DUCKPRI` write routed to a stopped target in the same block
    //    as `target = 11` sees the caller's new `target` during that refresh.
    let mut ducked = NOTE_ON_V1.to_vec();
    ducked.extend([SET_LOOP, LOADL, 5, 10, 0, 0, 0, WAIT, 5]);
    ducked.extend(loadl(25, 10)); // raise DUCKPRI above main_duckpri=5 (must stay latched!)
    ducked.extend(loadl(26, 1)); // toggle Is3d (must stay latched!)
    ducked.extend([SET_LOOP, LOOP]);

    let mut set_duck_5 = loadl(123, 5);
    set_duck_5.extend([SET_LOOP, LOOP]);

    let mut same_max_duck = loadl(123, 5);
    same_max_duck.push(END);

    let (hit, addresses) = assemble(&[&ducked, &set_duck_5, &same_max_duck]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,one,\"one.xa\"\n\
         [Track]\n\
         10=0,t10,1024,0,0,1,0,0x100,1\n\
         11=0,t11,1024,0,0,1,0,0x101,1\n\
         12=0,t12,1024,0,0,1,0,0x102,1\n\
         [EventMapping]\n\
         e0=1,10\n\
         e1=1,11\n\
         e2=1,12\n",
        addresses[0], addresses[1], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut player = Player::new(vec![module], long_sample("one.xa"), 1);

    play(&mut player, "e1"); // sets MAIN_DUCKPRI = 5
    play(&mut player, "e0"); // plays ducked (duckpri=0 < 5) -> half gain
    let initial = player.render(BLOCK, &NoSources)[0];
    assert!(initial > 0 && initial < 6_000, "{initial}");
    // Advance past the 10 ms wait so e0 writes DUCKPRI=10 and Is3d=1.
    // Neither refreshes the voice, so gain stays latched at `initial`.
    let after_writes = player.render(BLOCK * 6, &NoSources);
    assert_eq!(*after_writes.last().unwrap(), initial);
    // Track 12 writes MAIN_DUCKPRI=5 (maximum unchanged at 5): 0x57d31e
    // still refreshes all unheld running voices, lifting track 10 to full.
    play(&mut player, "e2");
    let refreshed = player.render(BLOCK, &NoSources)[0];
    assert_eq!(refreshed, 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // Part 2: same-block target switch + stopped-target MAIN_DUCKPRI write.
    let mut zero_11 = loadl(53, 11);
    zero_11.extend(loadl(19, 0));
    zero_11.push(END);

    let mut same_block = NOTE_ON_V1.to_vec();
    same_block.extend(loadl(53, 11)); // target stopped track 11 (EXTVOL=0)
    same_block.extend(loadl(123, 5)); // routed MAIN_DUCKPRI on stopped 11
    same_block.extend([SET_LOOP, LOOP]);

    let mut player2 = Player::new(
        vec![module_with(&[&same_block, &[END], &zero_11], 1, 10, "")],
        long_sample("one.xa"),
        1,
    );
    play(&mut player2, "e2");
    player2.render(BLOCK, &NoSources);
    play(&mut player2, "e0");
    // Refresh during MAIN_DUCKPRI sees e0's new target=11 (EXTVOL=0) immediately.
    assert!(player2.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    assert!(player2.faults.is_empty(), "{:?}", player2.faults);
}

#[test]
fn render_until_idle_ends_on_block_boundary_even_with_nudge_armed() {
    // `render_until_idle` always finishes on a `BLOCK` (64-frame) boundary
    // even when the 20 ms nudge timer (`TIMER_FRAMES = 441`) fires mid-block.
    let mut map = BTreeMap::new();
    map.insert(
        "one.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![1000; 440],
        },
    );
    let generic: &[u8] = &[NOTE_ON_V1[0], NOTE_ON_V1[1], WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic], 1)], map, 1);
    player.set_active(true, &NoSources); // arms 20 ms nudge countdown
    play(&mut player, "e0");
    let out = player.render_until_idle(RATE as usize, &NoSources);
    // Note ends at frame 440; `render_until_idle` stops at frame 448 (7 * 64),
    // NOT at the mid-block timer tick at frame 441.
    assert_eq!(out.len(), 7 * BLOCK);
    assert!(player.is_idle());
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn natural_track_end_clears_target_before_ducking_refresh() {
    // `0x57dd87..0x57dda5` clears `target` and removes an ending track from
    // the running list BEFORE `0x57ddd8` refreshes surviving voices.
    // Track 10 plays a note and targets track 11; track 11 has EXTVOL=0 and
    // targets track 12 (EXTVOL=1024), then ends naturally after 10 ms.
    // While track 11 runs, track 10's target chain resolves through 11 -> 12
    // (EXTVOL=1024). When track 11 ends, its `target` clears and it becomes
    // stopped BEFORE the kill's voice refresh runs, so track 10 resolves to
    // stopped track 11 (EXTVOL=0) and falls silent immediately.
    let mut t10 = NOTE_ON_V1.to_vec();
    t10.extend(loadl(53, 11));
    t10.extend([SET_LOOP, LOOP]);

    let mut t11 = loadl(19, 0); // local EXTVOL = 0 on track 11
    t11.extend(loadl(53, 12)); // forward to track 12 (EXTVOL = 1024)
    t11.extend(loadl(5, 10));
    t11.extend([WAIT, 5, END]);

    let t12: &[u8] = &[SET_LOOP, LOOP];

    let mut player = Player::new(
        vec![module_with(&[&t10, &t11, t12], 1, 10, "")],
        long_sample("one.xa"),
        1,
    );
    play(&mut player, "e2"); // start track 12 (EXTVOL=1024)
    play(&mut player, "e1"); // start track 11 (local EXTVOL=0, target=12)
    play(&mut player, "e0"); // start track 10 (target=11)
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    // Advance past track 11's 10 ms wait so track 11 ends naturally.
    let _ = player.render(BLOCK * 5, &NoSources);
    assert!(
        player.render(BLOCK, &NoSources).iter().all(|&s| s == 0),
        "track 10 should refresh against stopped track 11 (EXTVOL=0) when track 11 ends"
    );
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kill_trk_sees_caller_same_block_duckpri_update_during_refresh() {
    // Caller (track 10) plays ducked under MAIN_DUCKPRI=5, then in one block
    // raises its own DUCKPRI to 10 (register 25 — latched) and calls
    // `kill_trk` on stopped track 12. The kill's `0x57d31e` voice refresh
    // must see track 10's new DUCKPRI=10 and lift its gain to full (10_000).
    let mut t10 = NOTE_ON_V1.to_vec();
    t10.extend(loadl(5, 10));
    t10.extend([WAIT, 5]);
    t10.extend(loadl(25, 10)); // raise own DUCKPRI above 5
    t10.extend(loadl(6, 12));
    t10.extend([KILL_TRK, 6, SET_LOOP, LOOP]);

    let mut ducker = loadl(123, 5);
    ducker.extend([SET_LOOP, LOOP]);

    let mut player = Player::new(
        vec![module_with(&[&t10, &ducker, &[END]], 1, 10, "")],
        long_sample("one.xa"),
        1,
    );
    play(&mut player, "e1"); // MAIN_DUCKPRI = 5
    play(&mut player, "e0"); // plays ducked
    let initial = player.render(BLOCK, &NoSources)[0];
    assert!(initial > 0 && initial < 6_000, "{initial}");
    // Advance past the 10 ms wait so track 10 writes DUCKPRI=10 and runs kill_trk 12.
    let after = player.render(BLOCK * 6, &NoSources);
    assert_eq!(*after.last().unwrap(), 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // Killing an unused patch ID (1) or unregistered ID (999) dispatches to
    // patch vtable +0x38 (0x57bfa6 no-op) or NULL, so it must NOT refresh
    // unrelated latched voices.
    let mut kill_patch = NOTE_ON_V1.to_vec();
    kill_patch.extend(loadl(5, 10));
    kill_patch.extend([WAIT, 5]);
    kill_patch.extend(loadl(25, 10)); // raise DUCKPRI above 5 (latched)
    kill_patch.extend(loadl(6, 1)); // patch ID 1 (not a track!)
    kill_patch.extend([KILL_TRK, 6]);
    kill_patch.extend(loadl(6, 999)); // unregistered ID
    kill_patch.extend([KILL_TRK, 6, SET_LOOP, LOOP]);

    let mut player2 = Player::new(
        vec![module_with(&[&kill_patch, &ducker], 1, 10, "")],
        long_sample("one.xa"),
        1,
    );
    play(&mut player2, "e1"); // MAIN_DUCKPRI = 5
    play(&mut player2, "e0"); // plays ducked
    let ducked = player2.render(BLOCK, &NoSources)[0];
    assert!(ducked > 0 && ducked < 6_000, "{ducked}");
    let after_patch_kill = player2.render(BLOCK * 6, &NoSources);
    assert_eq!(
        *after_patch_kill.last().unwrap(),
        ducked,
        "killing a patch or unregistered ID must not refresh latched voices"
    );
    assert!(player2.faults.is_empty(), "{:?}", player2.faults);
}

#[test]
fn non_streamed_note_on_keeps_latched_gain_across_target_while_streamed_refreshes() {
    // `0x57f601..0x57f618`: `note_on` refreshes gains/pitch through `target`
    // only when the patch's `IsStreamed` flag (register 38) is set; a
    // non-streamed patch keeps the voice's latched gains from fresh start.
    let mut zero_11 = loadl(53, 11);
    zero_11.extend(loadl(19, 0));
    zero_11.push(END);

    let mut target_then_note = loadl(53, 11); // target stopped 11 (EXTVOL=0)
    target_then_note.extend(NOTE_ON_V1);
    target_then_note.extend([SET_LOOP, LOOP]);

    let (hit, addresses) = assemble(&[&target_then_note, &[END], &zero_11]);
    // Patch 1: IsStreamed = 0; Patch 2: IsStreamed = 1 (second flag after path);
    // Patch 3: IsStreamed = 1 in HOT, but path contains "sounddata\\" so 0x57b5d1 clears it.
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,mem,\"one.xa\",0,0,0,0,0\n\
         2=0,stream,\"stream.xa\",0,1,0,0,0\n\
         3=0,sd,\"SoundData\\sd.xa\",0,1,0,0,0\n\
         [Track]\n\
         10=0,mem_trk,1024,0,0,1,0,0x100,1\n\
         11=0,stopped,1024,0,0,1,0,0x101,1\n\
         12=0,zero,1024,0,0,0,0,0x102,1\n\
         13=0,stream_trk,1024,0,0,1,0,0x100,2\n\
         14=0,sd_trk,1024,0,0,1,0,0x100,3\n\
         [EventMapping]\n\
         mem=1,10\n\
         zero=1,12\n\
         stream=1,13\n\
         sd=1,14\n",
        addresses[0], addresses[1], addresses[2]
    );
    let module = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
    let mut map = long_sample("one.xa");
    map.insert(
        "stream.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![10_000; 20 * BLOCK],
        },
    );
    map.insert(
        "SoundData\\sd.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![10_000; 20 * BLOCK],
        },
    );
    let mut player = Player::new(vec![module], map, 1);

    play(&mut player, "zero");
    player.render(BLOCK, &NoSources);

    // Non-streamed patch 1 keeps its fresh-start latched gain (10_000).
    play(&mut player, "mem");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    player.handle(&SoundEvent::StopAll, &NoSources);

    // Streamed patch 2 refreshes through target 11 (EXTVOL=0) -> silent.
    play(&mut player, "zero");
    player.render(BLOCK, &NoSources);
    play(&mut player, "stream");
    assert!(player.render(BLOCK, &NoSources).iter().all(|&s| s == 0));
    player.handle(&SoundEvent::StopAll, &NoSources);

    // Patch 3 has "SoundData\\" in its path, so 0x57b5d1 clears IsStreamed -> 10_000.
    play(&mut player, "zero");
    player.render(BLOCK, &NoSources);
    play(&mut player, "sd");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn same_block_ducking_write_and_kill_refresh_latched_gain_before_non_streamed_note_on() {
    // `0x57e011 -> 0x57d31e -> 0x57cc74 -> 0x57f759` and `0x57ddd8 -> 0x57f759`
    // synchronously refresh the running track's latched voice gains before a
    // same-block non-streamed `note_on` (`0x57f601..0x57f61d`).
    //
    // Track 10: sets own MAIN_DUCKPRI = 5, then immediately runs non-streamed
    // `note_on`. With default DUCKPRI = 0 < 5, gain ducks to 10% (999 PCM).
    let mut duck_then_note = loadl(123, 5);
    duck_then_note.extend(NOTE_ON_V1);
    duck_then_note.extend([SET_LOOP, LOOP]);

    // Track 11: routes MAIN_DUCKPRI = 5 onto stopped track 12 (stays ducked).
    let mut set_stopped_duck = loadl(53, 12);
    set_stopped_duck.extend(loadl(123, 5));
    set_stopped_duck.push(END);

    // Track 13: clears stopped track 12's MAIN_DUCKPRI = 0 via target, then
    // immediately runs non-streamed `note_on` -> refreshed to unducked 10_000.
    let mut clear_stopped_then_note = loadl(53, 12);
    clear_stopped_then_note.extend(loadl(123, 0));
    clear_stopped_then_note.extend(NOTE_ON_V1);
    clear_stopped_then_note.extend([SET_LOOP, LOOP]);

    // Track 14: running ducker (MAIN_DUCKPRI = 5, loops forever).
    let mut running_ducker = loadl(123, 5);
    running_ducker.extend([SET_LOOP, LOOP]);

    // Track 15: kills running ducker 14 via `kill_trk`, then immediately runs
    // non-streamed `note_on` -> refreshed to unducked 10_000.
    let mut kill_running_then_note = loadl(5, 14);
    kill_running_then_note.extend([KILL_TRK, 5]);
    kill_running_then_note.extend(NOTE_ON_V1);
    kill_running_then_note.extend([SET_LOOP, LOOP]);

    // Track 16: kills stopped ducker 12 via `kill_trk`, then immediately runs
    // non-streamed `note_on` -> refreshed to unducked 10_000.
    let mut kill_stopped_then_note = loadl(5, 12);
    kill_stopped_then_note.extend([KILL_TRK, 5]);
    kill_stopped_then_note.extend(NOTE_ON_V1);
    kill_stopped_then_note.extend([SET_LOOP, LOOP]);

    let (hit, addresses) = assemble(&[
        &duck_then_note,
        &set_stopped_duck,
        &[END],
        &clear_stopped_then_note,
        &running_ducker,
        &kill_running_then_note,
        &kill_stopped_then_note,
    ]);
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         0x103={:#x}\n\
         0x104={:#x}\n\
         0x105={:#x}\n\
         0x106={:#x}\n\
         [Patch]\n\
         1=0,mem,\"one.xa\",0,0,0,0,0\n\
         [Track]\n\
         10=0,duck_note,1024,0,0,1,0,0x100,1\n\
         11=0,set_stopped,1024,0,0,1,0,0x101,1\n\
         12=0,stopped_ducker,1024,0,0,1,0,0x102,1\n\
         13=0,clear_stopped,1024,0,0,1,0,0x103,1\n\
         14=0,running_ducker,1024,0,0,1,0,0x104,1\n\
         15=0,kill_running,1024,0,0,1,0,0x105,1\n\
         16=0,kill_stopped,1024,0,0,1,0,0x106,1\n\
         [EventMapping]\n\
         duck_note=1,10\n\
         set_stopped=1,11\n\
         clear_stopped=1,13\n\
         running_ducker=1,14\n\
         kill_running=1,15\n\
         kill_stopped=1,16\n",
        addresses[0],
        addresses[1],
        addresses[2],
        addresses[3],
        addresses[4],
        addresses[5],
        addresses[6],
    );
    let make = || {
        let m = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
        Player::new(vec![m], long_sample("one.xa"), 1)
    };

    // Case 1: own MAIN_DUCKPRI = 5 immediately before non-streamed note_on.
    let mut player = make();
    play(&mut player, "duck_note");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 999);
    assert_eq!(player.render(BLOCK, &NoSources)[0], 999);
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // Case 2: routed MAIN_DUCKPRI = 0 on stopped track immediately before note_on.
    let mut player = make();
    play(&mut player, "set_stopped");
    player.render(BLOCK, &NoSources);
    play(&mut player, "clear_stopped");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // Case 3: kill_trk on running ducker immediately before non-streamed note_on.
    let mut player = make();
    play(&mut player, "running_ducker");
    player.render(BLOCK, &NoSources);
    play(&mut player, "kill_running");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    // Case 4: kill_trk on stopped ducker immediately before non-streamed note_on.
    let mut player = make();
    play(&mut player, "set_stopped");
    player.render(BLOCK, &NoSources);
    play(&mut player, "kill_stopped");
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert_eq!(player.render(BLOCK, &NoSources)[0], 10_000);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn patch_write_copies_is3d_and_flags_before_refreshing_gains() {
    // `0x57f6e3..0x57f6fb` copies the replacement patch's `Is3d` flag into
    // track register `THREE_D` (26) via `vtable +0x84` (setting comparison
    // flags from `is_3d` at `0x57de98..0x57dec5`) before `0x57e04b` refreshes
    // gains.
    //
    // Track 10 starts with 3D patch 1 (Is3d=1), sets PAN=0 (hard left), then
    // replaces PATCH=2 (2D patch, Is3d=0) and runs non-streamed `note_on`.
    // Because Is3d became 0 before the gain refresh, the note plays hard-left
    // ([10_000, 0]) instead of 3D-centered ([10_000, 10_000]).
    let mut replace_3d_with_2d = loadl(20, 0); // PAN = 0 (hard left)
    replace_3d_with_2d.extend(loadl(50, 2)); // PATCH = 2 (Is3d = 0)
    replace_3d_with_2d.extend(NOTE_ON_V1);
    replace_3d_with_2d.extend([SET_LOOP, LOOP]);

    // Track 11 writes PATCH=2 (Is3d=0, so comparison flags become zero) and
    // checks `ifeq` -> jumps to 0x102 which plays `note_on`.
    const IFEQ: u8 = 62;
    let mut patch_then_ifeq = loadl(50, 2);
    patch_then_ifeq.push(IFEQ);
    patch_then_ifeq.extend(long(0x102));
    patch_then_ifeq.push(END);

    let mut branch_taken = Vec::from(NOTE_ON_V1);
    branch_taken.extend([SET_LOOP, LOOP]);

    let (hit, addresses) = assemble(&[&replace_3d_with_2d, &patch_then_ifeq, &branch_taken]);
    // Patch 1: 4th flag after path = 1 (`Is3d = 1`).
    // Patch 2: 4th flag after path = 0 (`Is3d = 0`).
    let hot = format!(
        "[EventMappingEquate]\n\
         [TrackData]\n\
         0x100={:#x}\n\
         0x101={:#x}\n\
         0x102={:#x}\n\
         [Patch]\n\
         1=0,p3d,\"one.xa\",0,0,0,1,0\n\
         2=0,p2d,\"two.xa\",0,0,0,0,0\n\
         [Track]\n\
         10=0,pan_test,1024,0,0,1,0,0x100,1\n\
         11=0,flag_test,1024,0,0,1,0,0x101,1\n\
         [EventMapping]\n\
         pan=1,10\n\
         flag=1,11\n",
        addresses[0], addresses[1], addresses[2],
    );
    let mut map = long_sample("one.xa");
    map.insert(
        "two.xa".to_owned(),
        Pcm {
            rate: RATE,
            channels: 1,
            samples: vec![10_000; 20 * BLOCK],
        },
    );
    let make = || {
        let m = Module::parse("test", &hit, b"v1 5\n", hot.as_bytes()).unwrap();
        Player::new(vec![m], map.clone(), 1)
    };

    let mut player = make();
    play(&mut player, "pan");
    let out = player.render_stereo(BLOCK, &NoSources);
    assert_eq!((out[0], out[1]), (10_000, 0));
    assert!(player.faults.is_empty(), "{:?}", player.faults);

    let mut player = make();
    play(&mut player, "flag");
    let out = player.render_stereo(BLOCK, &NoSources);
    assert_eq!((out[0], out[1]), (10_000, 10_000));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}
