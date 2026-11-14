//! The HIT player on hand-built modules and samples.

use sims_audio::levels;
use sims_audio::module::Module;
use sims_audio::player::{BLOCK, NoSources, Player, RATE, Sources};
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
    let (hit, addresses) = assemble(routines);
    let mut hot = String::from("[EventMappingEquate]\nkSndobPlay=1\n[TrackData]\n");
    for (i, address) in addresses.iter().enumerate() {
        hot.push_str(&format!("{:#x}={address:#x}\n", 0x100 + i));
    }
    hot.push_str("[Patch]\n1=0,one,\"one.xa\"\n2=0,two,\"two.xa\"\n");
    hot.push_str("[GlobalHitList]\n5=1,2\n[Track]\n");
    for i in 0..routines.len() {
        hot.push_str(&format!(
            "{}=0,t{i},1024,2,0,0,0,{:#x},{patch}\n",
            10 + i,
            0x100 + i
        ));
    }
    hot.push_str("[EventMapping]\n");
    for i in 0..routines.len() {
        hot.push_str(&format!("e{i}=kSndobPlay,{}\n", 10 + i));
    }
    hot.push_str("other=7,10\n");
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
    fn field(&self, object: u32, source: i32, field: i32) -> i32 {
        assert_eq!((object, source, field), (9, 9, 3));
        self.0
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
    assert!(player.faults[1].contains("kind Some(7)"));
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
