//! The HIT player on hand-built modules and samples.

use sims_audio::levels;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE, Sources};
use sims_audio::sources::Snapshot;
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
    player.handle(&SoundEvent::Play {
        object: 9,
        name: name.into(),
    });
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
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic], 1)], map, 1);
    play(&mut player, "e0");
    play(&mut player, "e0");
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
    player.handle(&SoundEvent::StopObject { object: 9 });
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
    player.handle(&SoundEvent::Pause);
    assert!(player.is_paused());
    // Held: silent, and the thread waits, however long.
    assert!(player.render(3 * BLOCK, &NoSources).iter().all(|&s| s == 0));
    assert!(!player.is_idle());
    player.handle(&SoundEvent::Unpause);
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
    player.handle(&SoundEvent::Pause);
    assert_eq!(
        player.render(10, &NoSources),
        (11..=20).collect::<Vec<i16>>()
    );
    // A track started while paused plays: starting it lets the track
    // go, so its held note goes on at once, and the new thread's note
    // starts when the threads next run, at the next block.
    play(&mut player, "e0");
    let out = player.render(10, &NoSources);
    let resumed: Vec<i16> = (11..=20).map(|v| v + v + 10).collect();
    assert_eq!(out, resumed);
    player.handle(&SoundEvent::Unpause);
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
    player.handle(&SoundEvent::StopAll);
}

const LOADB_V1: u8 = 4;
const PLAY_TRK: u8 = 50;

#[test]
fn a_track_that_restarts_itself_runs_once_per_block() {
    // Track 10: v1 = 10; play_trk v1; end. Each run starts its own
    // replacement, which first runs in the next block, so `render`
    // returns and one thread is left at a time.
    let respawn: &[u8] = &[LOADB_V1, 5, 10, PLAY_TRK, 5, END];
    let mut player = Player::new(vec![module(&[respawn], 0)], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(1, &NoSources), [0]);
    assert_eq!(player.render(20 * BLOCK, &NoSources).len(), 20 * BLOCK);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
    assert!(!player.is_idle());
    player.handle(&SoundEvent::StopAll);
    assert!(player.is_idle());
}

#[test]
fn threads_that_multiply_stop_at_the_thread_limit() {
    // Each run starts two copies of itself: 1, 2, 4, ... threads.
    let fork: &[u8] = &[LOADB_V1, 5, 10, PLAY_TRK, 5, PLAY_TRK, 5, END];
    let mut player = Player::new(vec![module(&[fork], 0)], samples(), 1);
    play(&mut player, "e0");
    player.render(40 * BLOCK, &NoSources);
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
    let spin: &[u8] = &[13, 0, 1, 0, 0]; // jump 0x100: itself
    let mut player = Player::new(vec![module(&[spin], 0)], samples(), 1);
    for _ in 0..12 {
        play(&mut player, "e0");
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
    player.handle(&SoundEvent::StopAll);
    play(&mut player, "e0");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    // From the next block.
    assert_eq!(&out[BLOCK..BLOCK + 100], &(1..=100).collect::<Vec<i16>>());
    // Two threads of one track do not end each other: a group holds a
    // track number. Both play.
    player.handle(&SoundEvent::StopAll);
    play(&mut player, "e1");
    play(&mut player, "e1");
    let out = player.render_until_idle(10 * BLOCK, &NoSources);
    assert_eq!(out.iter().find(|&&s| s != 0), Some(&2));
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
    // e0 claims group 1, waits until its extvol (19) reaches fade_dest
    // (1, register 8 here), then plays patch 1.
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
    // by 200 a block from 1024, reaching 1 in the seventh block; then its
    // note plays at extvol 1 of 1024: -39.97 dB, about a hundredth.
    assert!(out[..BLOCK].iter().all(|&s| s == 0));
    assert_eq!(&out[BLOCK..BLOCK + 3], [2000, 4000, -2000]);
    assert!(out[BLOCK + 3..6 * BLOCK].iter().all(|&s| s == 0));
    let gain = i64::from(levels::gain(levels::attenuation(1, 1024)));
    assert_eq!(levels::attenuation(1, 1024), -3997);
    let faint: Vec<i16> = (1..=100).map(|s| ((s * gain) >> 16) as i16).collect();
    assert_eq!(&out[6 * BLOCK..6 * BLOCK + 100], &faint);
    assert!(player.is_idle());
}

#[test]
fn the_first_module_defining_an_event_wins_and_play_from_picks_one() {
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let first = module(&[generic], 1);
    let second = module(&[generic], 2);
    let mut player = Player::new(vec![first, second], samples(), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(3, &NoSources), [1, 2, 3]);
    player.handle(&SoundEvent::StopAll);
    player.play_from(1, 9, "e0");
    player.render(BLOCK - 3, &NoSources);
    assert_eq!(player.render(3, &NoSources), [2000, 4000, -2000]);
    player.handle(&SoundEvent::StopAll);
    player.play_from(2, 9, "e0");
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
    let mut player = Player::new(vec![module(&[&busy, generic], 1)], samples(), 1);
    for _ in 0..10 {
        play(&mut player, "e0");
    }
    play(&mut player, "e1");
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
    player.handle(&SoundEvent::Play {
        object: 1,
        name: "e0".into(),
    });
    play(&mut player, "e1");
    assert_eq!(player.render(2, &NoSources), [999, 999]);
    player.render(BLOCK - 2, &NoSources);
    // When the ducking thread stops, its request goes with it.
    player.handle(&SoundEvent::StopObject { object: 1 });
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
fn a_thread_s_last_writes_reach_its_notes_when_it_ends() {
    // note_on, then vol 0, then end: the note plays on, silent.
    let mut routine = NOTE_ON_V1.to_vec();
    routine.extend(loadl(18, 0));
    routine.push(END);
    let mut player = Player::new(vec![module(&[&routine], 1)], loud(4), 1);
    play(&mut player, "e0");
    assert_eq!(player.render(2, &NoSources), [0, 0]);
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
    player.handle(&SoundEvent::Play {
        object,
        name: name.into(),
    });
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

#[test]
fn an_object_that_goes_fades_out_its_instances() {
    // Objects 1 and 2 play track 10; object 1 goes. Its note fades by 100
    // a block, from 1024, and ends; object 2's plays on.
    let module = module_with(&[&LOOPING], 1, 10, "");
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play_for(&mut player, 1, "e0");
    play_for(&mut player, 2, "e0");
    player.render(BLOCK, &NoSources);
    player.handle(&SoundEvent::StopObject { object: 1 });
    let first = player.render(BLOCK, &NoSources);
    assert!(first[0] > 10_000 && first[0] < 20_000, "{}", first[0]);
    player.render(11 * BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [10_000]);
    // The last object going takes the track with it.
    player.handle(&SoundEvent::StopObject { object: 2 });
    player.render(12 * BLOCK, &NoSources);
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
    player.handle(&SoundEvent::StopObject { object: 1 });
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
    player.handle(&SoundEvent::StopObject { object: 3 });
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
    let events = "all=20,0\nobjects=105,0\n";
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
    // An object of 0 records no instance: its track plays on.
    let module = module_with(&[&LOOPING], 1, 10, events);
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play_for(&mut player, 0, "e0");
    player.render(BLOCK, &NoSources);
    play(&mut player, "objects");
    player.render(12 * BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [10_000]);
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn kinds_that_do_nothing_and_kinds_not_ported() {
    let events = "debug=44,0\nempty=29,10\nnone=1907,10\nmusic=36,0,1\npiano=43,10\n";
    let module = module_with(&[&GENERIC], 1, 10, events);
    let mut player = Player::new(vec![module], samples(), 1);
    for name in ["debug", "empty", "none"] {
        play(&mut player, name);
    }
    assert!(player.faults.is_empty() && player.is_idle());
    play(&mut player, "music");
    play(&mut player, "piano");
    assert_eq!(player.faults.len(), 2);
    assert!(player.faults[0].contains("kind 36 is not one"));
    assert!(player.faults[1].contains("kind 43 is not one"));
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
fn an_object_that_goes_stops_notes_its_ended_threads_left() {
    // e0 starts a note and ends; the note plays on without its thread.
    let routine: &[u8] = &[2, 5, END];
    let module = module_with(&[routine], 1, 10, "");
    let mut player = Player::new(vec![module], loud(10 * BLOCK), 1);
    play_for(&mut player, 1, "e0");
    player.render(BLOCK, &NoSources);
    // The track still plays, so object 1 stays its instance when object
    // 2 plays it too.
    play_for(&mut player, 2, "e0");
    player.render(BLOCK, &NoSources);
    assert_eq!(player.render(1, &NoSources), [20_000]);
    player.handle(&SoundEvent::StopObject { object: 1 });
    assert_eq!(player.render(1, &NoSources), [10_000]);
    player.handle(&SoundEvent::StopObject { object: 2 });
    assert!(player.is_idle());
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
