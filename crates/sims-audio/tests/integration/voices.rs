use sims_audio::Pcm;
use sims_audio::levels::BASE_RATE;
use sims_audio::voices::{
    MAX_OUTPUT_RATE, VoiceAgeGender, VoiceCatalog, VoicePitch, VoiceQuery, VoiceStream,
    resample_pitched,
};
use std::path::PathBuf;

fn sims_install() -> Option<PathBuf> {
    crate::common::install("the install's voices")
}

#[test]
fn voice_pitch_semitones_and_resample_behave_deterministically() {
    let native = VoicePitch::NATIVE;
    assert_eq!(native.playback_rate(), BASE_RATE);

    let octave_up = VoicePitch::from_semitones(12.0);
    assert_eq!(octave_up.playback_rate(), BASE_RATE * 2);

    let octave_down = VoicePitch::from_semitones(-12.0);
    assert_eq!(octave_down.playback_rate(), BASE_RATE / 2);

    // NaN clamps cleanly to native pitch.
    assert_eq!(VoicePitch::from_semitones(f32::NAN), VoicePitch::NATIVE);

    let pcm = Pcm {
        rate: BASE_RATE,
        channels: 1,
        samples: (0..220).map(|i| (i * 100) as i16).collect(),
    };

    // Native pitch at same output rate returns exact clone.
    let same = resample_pitched(&pcm, VoicePitch::NATIVE, BASE_RATE);
    assert_eq!(same, pcm);

    // Octave up halves duration (plays twice as fast).
    let faster = resample_pitched(&pcm, octave_up, BASE_RATE);
    assert_eq!(faster.rate, BASE_RATE);
    assert_eq!(faster.channels, 1);
    assert_eq!(faster.frames(), pcm.frames() / 2);

    // Octave down doubles duration (plays half as fast).
    let slower = resample_pitched(&pcm, octave_down, BASE_RATE);
    assert_eq!(slower.frames(), pcm.frames() * 2);

    // Extreme output_rate clamps to MAX_OUTPUT_RATE instead of attempting GiB allocation.
    let bounded = resample_pitched(&pcm, VoicePitch::NATIVE, u32::MAX);
    assert_eq!(bounded.rate, MAX_OUTPUT_RATE);
    assert!(bounded.samples.len() <= 25_000);
}

#[test]
fn voice_stream_reads_chunks_and_resets_cleanly() {
    let pcm = Pcm {
        rate: BASE_RATE,
        channels: 2,
        samples: vec![1, -1, 2, -2, 3, -3, 4, -4, 5, -5],
    };
    let mut stream = VoiceStream::new(pcm.clone());
    assert_eq!(stream.total_frames(), 5);
    assert_eq!(stream.remaining_frames(), 5);
    assert!(!stream.is_finished());

    let mut buf = [0i16; 6]; // 3 stereo frames
    assert_eq!(stream.read_frames(&mut buf), 3);
    assert_eq!(buf, [1, -1, 2, -2, 3, -3]);
    assert_eq!(stream.remaining_frames(), 2);

    let mut tail = [0i16; 6];
    assert_eq!(stream.read_frames(&mut tail), 2);
    assert_eq!(&tail[..4], &[4, -4, 5, -5]);
    assert!(stream.is_finished());
    assert_eq!(stream.read_frames(&mut tail), 0);

    stream.reset();
    assert_eq!(stream.remaining_frames(), 5);
    assert!(!stream.is_finished());
}

#[test]
fn complete_collection_voice_catalog_indexes_and_decodes_all_voice_banks() {
    let Some(root) = sims_install() else {
        return;
    };
    let catalog = VoiceCatalog::scan(&root).expect("scan Complete Collection voice bank");

    // Complete Collection ships >10,000 Simlish voice clips across SoundData + ExpansionPack*.
    assert!(
        catalog.len() >= 10_000,
        "expected >= 10,000 voice clips in Complete Collection, got {}",
        catalog.len()
    );
    assert!(
        !catalog.script_sets().is_empty(),
        "expected at least one voice HIT/HOT script set (e.g. repair_vox2)"
    );

    // Every major voice bucket must be populated.
    for bucket in [
        VoiceAgeGender::AdultMale,
        VoiceAgeGender::AdultFemale,
        VoiceAgeGender::Child,
        VoiceAgeGender::Baby,
        VoiceAgeGender::Dog,
        VoiceAgeGender::Cat,
        VoiceAgeGender::Npc,
    ] {
        let count = catalog.count_for(bucket);
        assert!(
            count > 0,
            "expected non-zero voice clips for {bucket:?}, got 0"
        );
    }

    // Multiple packs discovered (base SoundData + ExpansionPack5/6/7 + ExpansionShared).
    let packs = catalog.packs();
    for expected_pack in [
        "sounddata",
        "expansionshared",
        "expansionpack5",
        "expansionpack6",
        "expansionpack7",
    ] {
        assert!(
            packs.contains(&expected_pack),
            "expected pack {expected_pack} in voice catalog packs: {packs:?}"
        );
    }

    // Deterministic lookup and XA/WAV decode + pitched streaming across voice buckets.
    for bucket in [
        VoiceAgeGender::AdultMale,
        VoiceAgeGender::AdultFemale,
        VoiceAgeGender::Child,
        VoiceAgeGender::Dog,
        VoiceAgeGender::Cat,
        VoiceAgeGender::Npc,
    ] {
        let clip_a = catalog
            .pick(bucket, None, 42)
            .unwrap_or_else(|| panic!("no clip picked for {bucket:?}"));
        let clip_b = catalog
            .pick(bucket, None, 42)
            .unwrap_or_else(|| panic!("no clip picked on repeat for {bucket:?}"));
        assert_eq!(clip_a.key, clip_b.key);

        let resolved = catalog
            .resolve_path(&clip_a.path)
            .expect("resolve_path round-trip");
        assert_eq!(resolved.key, clip_a.key);

        let pcm = catalog
            .decode_clip(clip_a)
            .unwrap_or_else(|e| panic!("failed to decode {}: {e}", clip_a.path));
        assert!(pcm.frames() > 0, "empty PCM for {}", clip_a.path);
        assert!(pcm.rate > 0);
        assert!(matches!(pcm.channels, 1 | 2));

        let mut stream = catalog
            .open_stream(clip_a, VoicePitch::from_semitones(2.0), BASE_RATE)
            .expect("open pitched voice stream");
        let mut chunk = vec![0i16; 256 * usize::from(stream.channels())];
        let read = stream.read_frames(&mut chunk);
        assert!(read > 0);
    }

    // Also verify at least one WAV voice clip decodes cleanly (ExpansionPack7 ships .WAV voices).
    let wav_clip = catalog
        .clips()
        .iter()
        .find(|c| c.extension == "wav")
        .expect("expected at least one .WAV voice clip in Complete Collection");
    let wav_pcm = catalog
        .decode_clip(wav_clip)
        .expect("decode Complete Collection WAV voice clip");
    assert!(wav_pcm.frames() > 0);

    // Search query filtering by category & pack works.
    let laugh_clips = catalog.search(&VoiceQuery {
        voice: Some(VoiceAgeGender::AdultFemale),
        category: Some("laugh"),
        pack: Some("sounddata"),
        ..VoiceQuery::default()
    });
    assert!(
        !laugh_clips.is_empty(),
        "expected AdultFemale laugh clips in SoundData"
    );

    // Magic Town dragon NPC clips stay in the NPC bucket (not pet Cat/Dog buckets).
    for dragon_path in [
        "ExpansionPack7\\Sound\\npc\\dragon_cat_vox1.XA",
        "ExpansionPack7\\Sound\\npc\\dragon_dog_vox1.XA",
    ] {
        let clip = catalog
            .resolve_path(dragon_path)
            .unwrap_or_else(|| panic!("expected {dragon_path} in catalog"));
        assert_eq!(clip.voice, VoiceAgeGender::Npc);
    }
}
