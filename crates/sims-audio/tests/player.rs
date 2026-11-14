//! The HIT player on hand-built modules and samples.

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
            "{}=0,t{i},16,2,0,0,0,{:#x},{patch}\n",
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
fn slower_stereo_samples_are_mixed_down_and_interpolated() {
    let generic: &[u8] = &[2, 5, WAIT_SAMP, END];
    let mut player = Player::new(vec![module(&[generic], 2)], samples(), 1);
    play(&mut player, "e0");
    let out = player.render(8, &NoSources);
    // Frames 2000, 4000, -2000 at half the rate: every other output frame
    // is halfway between two; the last frame holds.
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
