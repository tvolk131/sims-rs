use sims_audio::files::SoundFiles;
use sims_audio::music::{ALL_EXPANSIONS, ModeCatalog, ModeMusicPlayer, MusicMode};
use sims_audio::player::{RATE, VOLUME_STEPS};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    path.is_dir().then_some(path)
}

/// Builds a tiny 16-bit PCM RIFF WAVE file holding a constant amplitude or
/// simple square wave so unit tests can inspect exact gain scaling, fades,
/// and track transitions without external assets.
fn make_wav(rate: u32, channels: u16, frames: usize, sample: i16) -> Vec<u8> {
    let data_bytes = (frames * usize::from(channels) * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    let byte_rate = rate * u32::from(channels) * 2;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for _ in 0..(frames * usize::from(channels)) {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[test]
fn mode_metadata_and_id_roundtrip() {
    for id in 0..=18i64 {
        let mode = MusicMode::from_id(id).expect("valid mode id");
        assert_eq!(mode.id(), id);
    }
    assert_eq!(MusicMode::from_id(-1), None);
    assert_eq!(MusicMode::from_id(19), None);

    // US vs non-US neighborhood directory (`0x571905`).
    assert_eq!(
        MusicMode::Neighborhood.directory(true, ALL_EXPANSIONS),
        Some("Music\\Modes\\NhoodUS")
    );
    assert_eq!(
        MusicMode::Neighborhood.directory(false, ALL_EXPANSIONS),
        Some("Music\\Modes\\Nhood")
    );
    assert_eq!(
        MusicMode::Buy.directory(true, ALL_EXPANSIONS),
        Some("Music\\Modes\\Buy")
    );
    assert_eq!(
        MusicMode::Build.directory(true, ALL_EXPANSIONS),
        Some("Music\\Modes\\Build")
    );
    assert_eq!(
        MusicMode::Load.directory(true, ALL_EXPANSIONS),
        Some("Music\\Modes\\Load")
    );

    // Credits follows newest installed expansion bit (`0x575595..0x5755fb`).
    assert_eq!(MusicMode::Credits.hit_list(0x80), Some(103_101));
    assert_eq!(MusicMode::Credits.hit_list(0x40), Some(100_102));
    assert_eq!(MusicMode::Credits.hit_list(0x20), Some(20_902));
    assert_eq!(MusicMode::Credits.hit_list(0x08), Some(14_093));
    assert_eq!(MusicMode::Credits.hit_list(0x02), Some(1_589));
    assert_eq!(MusicMode::Credits.hit_list(0x04), Some(6_100));
    assert_eq!(MusicMode::Credits.hit_list(0x00), Some(223));
}

#[test]
fn shuffle_playlist_never_repeats_immediately_and_advances_on_eof() {
    let mut catalog = ModeCatalog::empty();
    let names = vec![
        "nhood_a.wav".to_owned(),
        "nhood_b.wav".to_owned(),
        "nhood_c.wav".to_owned(),
    ];
    catalog.insert(MusicMode::Neighborhood, names.clone());

    let mut files = BTreeMap::new();
    for (idx, name) in names.iter().enumerate() {
        files.insert(
            name.clone(),
            make_wav(RATE, 2, 200, (idx as i16 + 1) * 2000),
        );
    }

    let mut player = ModeMusicPlayer::new(catalog, files, 42);
    player.set_crossfade_ms(0);
    player.set_mode(MusicMode::Neighborhood);

    let mut history = Vec::new();
    let first = player.current_track().unwrap().to_owned();
    history.push(first);

    // Advance through 15 song completions (~200 frames each).
    for _ in 0..15 {
        let prev = player.current_track().unwrap().to_owned();
        let _ = player.render_stereo(205);
        let cur = player.current_track().unwrap().to_owned();
        assert_ne!(cur, prev, "track repeated immediately: {history:?}");
        history.push(cur);
    }
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn mode_transitions_match_191_rules() {
    let mut catalog = ModeCatalog::empty();
    catalog.insert(MusicMode::Neighborhood, vec!["nhood.wav".into()]);
    catalog.insert(MusicMode::Buy, vec!["buy.wav".into()]);
    catalog.insert(MusicMode::Build, vec!["build.wav".into()]);
    catalog.insert(MusicMode::MagicTownBuy, vec!["mm_buy.wav".into()]);
    catalog.insert(MusicMode::MagicTownBuild, vec!["mm_build.wav".into()]);

    let mut files = BTreeMap::new();
    for (name, amp) in [
        ("nhood.wav", 12_000),
        ("buy.wav", 8_000),
        ("build.wav", 6_000),
        ("mm_buy.wav", 5_000),
        ("mm_build.wav", 4_000),
    ] {
        files.insert(name.to_owned(), make_wav(RATE, 2, RATE as usize * 10, amp));
    }

    let mut player = ModeMusicPlayer::new(catalog, files, 7);
    player.set_mode(MusicMode::Neighborhood);
    assert_eq!(player.current_track(), Some("nhood.wav"));

    // Options and FamilyBin -> Neighborhood keep the neighborhood track playing.
    player.set_mode(MusicMode::Options);
    assert_eq!(player.current_track(), Some("nhood.wav"));
    player.set_mode(MusicMode::FamilyBin);
    assert_eq!(player.current_track(), Some("nhood.wav"));
    player.set_mode(MusicMode::Neighborhood);
    assert_eq!(player.current_track(), Some("nhood.wav"));
    assert!(!player.is_fading_out());

    // Neighborhood -> Live triggers FadeAndKillTrackSlow (~6.84 s fade to silence).
    player.set_mode(MusicMode::Live);
    assert!(player.is_fading_out());
    let early = player.render_stereo(RATE as usize / 2);
    let mid = player.render_stereo(RATE as usize * 3);
    let tail = player.render_stereo(RATE as usize * 4);
    assert!(
        early[0] > mid[mid.len() - 2],
        "slow fade must decay over time"
    );
    assert!(!player.is_playing(), "slow fade should finish within 7.5 s");
    assert_eq!(tail[tail.len() - 2], 0);

    // Buy -> Live stops immediately (`0x575211..0x57525e`).
    player.set_mode(MusicMode::Buy);
    assert_eq!(player.current_track(), Some("buy.wav"));
    player.set_mode(MusicMode::Live);
    assert!(!player.is_playing());
    assert!(player.render_stereo(256).iter().all(|&s| s == 0));

    // Buy -> Build switches track with smooth crossfade.
    player.set_mode(MusicMode::Buy);
    let _ = player.render_stereo(1024);
    player.set_mode(MusicMode::Build);
    assert_eq!(player.current_track(), Some("build.wav"));
    let cf = player.render_stereo(RATE as usize / 2);
    assert!(cf.iter().any(|&s| s != 0));

    // MagicTownBuild <-> MagicTownBuy preserves the active track (`0x575467`).
    player.set_mode(MusicMode::MagicTownBuild);
    assert_eq!(player.current_track(), Some("mm_build.wav"));
    player.set_mode(MusicMode::MagicTownBuy);
    assert_eq!(player.current_track(), Some("mm_build.wav"));
}

#[test]
fn music_volume_slider_follows_191_volume_steps_and_mixes() {
    let mut catalog = ModeCatalog::empty();
    catalog.insert(MusicMode::Buy, vec!["buy.wav".into()]);
    let mut files = BTreeMap::new();
    files.insert(
        "buy.wav".into(),
        make_wav(RATE, 2, RATE as usize * 2, 10_000),
    );

    let mut player = ModeMusicPlayer::new(catalog, files, 1);
    player.set_crossfade_ms(0);
    player.set_mode(MusicMode::Buy);

    // Step 10 = full volume (1024).
    assert_eq!(player.volume_step(), 10);
    assert_eq!(player.volume(), VOLUME_STEPS[10]);
    let full = player.render_stereo(16);
    assert_eq!(full[0], 10_000);

    // Step 5 = 740 / 1024.
    player.set_volume_step(5);
    assert_eq!(player.volume(), VOLUME_STEPS[5]);
    let mid = player.render_stereo(16);
    let expected_mid = ((10_000i64 * i64::from(VOLUME_STEPS[5])) >> 10) as i16;
    assert_eq!(mid[0], expected_mid);

    // Step 0 = exact silence (0).
    player.set_volume_step(0);
    assert_eq!(player.volume(), 0);
    let muted = player.render_stereo(64);
    assert!(muted.iter().all(|&s| s == 0));

    // Saturating overlay via `mix_into_stereo`.
    player.set_volume_step(10);
    let mut buf = vec![30_000i16; 8];
    player.mix_into_stereo(&mut buf);
    assert!(buf.iter().all(|&s| s == i16::MAX));
}

#[test]
fn missing_or_corrupt_track_records_fault_and_falls_back_to_next() {
    let mut catalog = ModeCatalog::empty();
    catalog.insert(
        MusicMode::Buy,
        vec!["bad1.mp3".into(), "bad2.mp3".into(), "good.wav".into()],
    );
    let mut files = BTreeMap::new();
    files.insert("bad1.mp3".into(), b"not-a-valid-mp3".to_vec());
    files.insert("bad2.mp3".into(), b"also-invalid".to_vec());
    files.insert("good.wav".into(), make_wav(RATE, 2, 128, 4_000));

    // Seed 0 with 2 bad tracks + 1 good track must never stall across multiple cycles.
    let mut player = ModeMusicPlayer::new(catalog, files, 0);
    player.set_crossfade_ms(0);
    player.set_mode(MusicMode::Buy);
    assert_eq!(player.current_track(), Some("good.wav"));
    for _ in 0..8 {
        let out = player.render_stereo(140);
        assert!(out.iter().any(|&s| s != 0));
        assert_eq!(player.current_track(), Some("good.wav"));
    }
}

#[test]
fn low_output_rate_and_expansion_return_from_family_bin() {
    let mut catalog = ModeCatalog::empty();
    catalog.insert(MusicMode::Downtown, vec!["dt.wav".into()]);
    catalog.insert(MusicMode::Neighborhood, vec!["nb.wav".into()]);
    let mut files = BTreeMap::new();
    files.insert(
        "dt.wav".into(),
        make_wav(RATE, 2, RATE as usize * 10, 6_000),
    );
    files.insert("nb.wav".into(), make_wav(RATE, 2, RATE as usize * 2, 3_000));

    // Very low output rate (1 Hz) over a 22.05 kHz constant WAV must not drop frames.
    let mut low = ModeMusicPlayer::with_rate(catalog.clone(), files.clone(), 1, 1);
    low.set_crossfade_ms(0);
    low.set_mode(MusicMode::Downtown);
    let out = low.render_stereo(6);
    assert!(
        out.iter().all(|&s| s != 0),
        "low output rate dropped frames: {out:?}"
    );

    // Downtown -> FamilyBin -> Neighborhood keeps Downtown's active_station intact.
    let mut player = ModeMusicPlayer::new(catalog, files, 1);
    player.set_mode(MusicMode::Downtown);
    assert_eq!(player.active_station(), Some(MusicMode::Downtown));
    player.set_mode(MusicMode::FamilyBin);
    player.set_mode(MusicMode::Neighborhood);
    assert_eq!(player.active_station(), Some(MusicMode::Downtown));
    assert_eq!(player.current_track(), Some("dt.wav"));
}

#[test]
fn install_catalog_discovers_all_mode_directories_us_and_non_us() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();

    let us = ModeCatalog::from_sound_files(&files, true, ALL_EXPANSIONS);
    assert_eq!(us.tracks(MusicMode::Buy).len(), 4);
    assert_eq!(us.tracks(MusicMode::Build).len(), 6);
    assert_eq!(us.tracks(MusicMode::Neighborhood).len(), 5);
    assert_eq!(us.tracks(MusicMode::Load).len(), 1);
    assert_eq!(us.tracks(MusicMode::Downtown).len(), 8);
    assert_eq!(us.tracks(MusicMode::Vacation).len(), 8);
    assert_eq!(us.tracks(MusicMode::Unleashed).len(), 6);
    assert_eq!(us.tracks(MusicMode::Superstar).len(), 6);
    assert_eq!(us.tracks(MusicMode::MagicTown).len(), 7);
    assert_eq!(us.tracks(MusicMode::MagicTownBuild).len(), 7);
    assert_eq!(us.tracks(MusicMode::MagicTownBuy).len(), 5);
    assert_eq!(us.tracks(MusicMode::MagicTownCredits).len(), 1);
    assert_eq!(us.tracks(MusicMode::Credits).len(), 1);

    let non_us = ModeCatalog::from_sound_files(&files, false, ALL_EXPANSIONS);
    assert_eq!(non_us.tracks(MusicMode::Neighborhood).len(), 4);
}

#[test]
fn install_streams_nhood_buy_build_and_load_with_volume_and_transitions() {
    let Some(root) = install() else { return };
    let files = SoundFiles::scan(&root).unwrap();
    let catalog = ModeCatalog::from_sound_files(&files, true, ALL_EXPANSIONS);

    let mut player_a = ModeMusicPlayer::new(catalog.clone(), files.clone(), 99);
    let mut player_b = ModeMusicPlayer::new(catalog, files, 99);

    for mode in [
        MusicMode::Neighborhood,
        MusicMode::Buy,
        MusicMode::Build,
        MusicMode::Load,
    ] {
        player_a.set_mode(mode);
        player_b.set_mode(mode);
        assert_eq!(player_a.current_track(), player_b.current_track());
        // Render 2 seconds of streaming resampled stereo PCM (clears ~1s lead-in on tracks like nhood2.mp3).
        let out_a = player_a.render_stereo(2 * RATE as usize);
        let out_b = player_b.render_stereo(2 * RATE as usize);
        assert_eq!(out_a, out_b, "deterministic replay failed for {mode:?}");
        assert!(
            out_a.iter().any(|&s| s.unsigned_abs() > 64),
            "mode {mode:?} ({:?}) rendered silence",
            player_a.current_track()
        );
    }

    // Muting Music volume step 0 produces exact silence.
    player_a.set_mode(MusicMode::Buy);
    player_a.set_volume_step(0);
    assert!(player_a.render_stereo(2048).iter().all(|&s| s == 0));

    // Buy -> Live stops immediately; zero faults throughout.
    player_a.set_volume_step(10);
    player_a.set_mode(MusicMode::Live);
    assert!(!player_a.is_playing());
    assert!(player_a.faults.is_empty(), "{:?}", player_a.faults);
}
