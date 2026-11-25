use sims_audio::files::{Entry, SoundFiles, Source};
use sims_audio::placement::Projection;
use sims_audio::player::{RATE, VOLUME_STEPS};
use sims_audio::radio::{
    BuiltinStation, RadioCatalog, RadioPlayer, SpatialAttenuation, StationId, StationKind,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    crate::common::install("the install's radio and TV")
}

fn make_wav(rate: u32, channels: u16, frames: usize, sample: i16) -> Vec<u8> {
    let data_bytes = (frames * usize::from(channels) * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
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
fn custom_station_and_builtin_discovery_from_sound_files() {
    let mut sf = SoundFiles::default();
    let add = |sf: &mut SoundFiles, rel: &str| {
        sf.insert(Entry {
            path: rel.to_owned(),
            source: Source::Loose(PathBuf::from(rel)),
        });
    };

    add(&mut sf, "Music\\Stations\\Rock\\rock1.mp3");
    add(&mut sf, "Music\\Stations\\Rock\\custom_user.mp3");
    add(&mut sf, "Music\\Stations\\Classica\\cmaj.mp3");
    add(&mut sf, "Music\\Stations\\Jazz\\miles.mp3");
    add(&mut sf, "Music\\Stations\\BossaNova\\ipanema.wav");
    add(&mut sf, "SoundData\\TVStations\\TV_Action\\a\\clip1.xa");
    add(&mut sf, "ExpansionPack6\\Sound\\TVStations\\News\\n1.xa");

    let catalog = RadioCatalog::from_sound_files(&sf);
    assert_eq!(
        catalog.tracks(&StationId::Builtin(BuiltinStation::Rock)),
        &[
            "Music\\Stations\\Rock\\custom_user.mp3".to_owned(),
            "Music\\Stations\\Rock\\rock1.mp3".to_owned(),
        ]
    );
    assert_eq!(
        catalog.tracks(&StationId::Builtin(BuiltinStation::Classical)),
        &["Music\\Stations\\Classica\\cmaj.mp3".to_owned()]
    );
    assert_eq!(
        catalog.tracks(&StationId::custom("Jazz")),
        &["Music\\Stations\\Jazz\\miles.mp3".to_owned()]
    );
    assert_eq!(
        catalog.tracks(&StationId::custom("BossaNova")),
        &["Music\\Stations\\BossaNova\\ipanema.wav".to_owned()]
    );
    assert_eq!(
        catalog.tracks(&StationId::Builtin(BuiltinStation::TvAction)),
        &["SoundData\\TVStations\\TV_Action\\a\\clip1.xa".to_owned()]
    );
    assert_eq!(
        catalog.tracks(&StationId::Builtin(BuiltinStation::TvNews)),
        &["ExpansionPack6\\Sound\\TVStations\\News\\n1.xa".to_owned()]
    );

    assert_eq!(
        catalog.station_by_name("classical"),
        Some(StationId::Builtin(BuiltinStation::Classical))
    );
    assert_eq!(
        catalog.station_by_name("JAZZ"),
        Some(StationId::custom("jazz"))
    );
    assert_eq!(
        catalog.station_for_hit_list(233),
        Some(StationId::Builtin(BuiltinStation::Rock))
    );
    assert_eq!(
        catalog.station_for_hit_list(529),
        Some(StationId::Builtin(BuiltinStation::TvAction))
    );
}

#[test]
fn shuffle_never_repeats_immediately_and_switches_stations() {
    let mut catalog = RadioCatalog::empty();
    let rock = vec![
        "rock_a.wav".to_owned(),
        "rock_b.wav".to_owned(),
        "rock_c.wav".to_owned(),
    ];
    let disco = vec!["disco_a.wav".to_owned(), "disco_b.wav".to_owned()];
    catalog.insert(BuiltinStation::Rock, rock.clone());
    catalog.insert(BuiltinStation::Disco, disco.clone());

    let mut files = BTreeMap::new();
    for (i, p) in rock.iter().enumerate() {
        files.insert(p.clone(), make_wav(RATE, 2, 160, (i as i16 + 1) * 3000));
    }
    for (i, p) in disco.iter().enumerate() {
        files.insert(p.clone(), make_wav(RATE, 2, 160, (i as i16 + 1) * 4000));
    }

    let mut player = RadioPlayer::new(catalog, files, 99);
    player.set_crossfade_ms(0);
    assert!(player.tune(10, BuiltinStation::Rock, SpatialAttenuation::FULL));

    let mut last = player.emitter_track(10).unwrap().to_owned();
    for _ in 0..12 {
        let _ = player.render_stereo(165);
        let cur = player.emitter_track(10).unwrap().to_owned();
        assert_ne!(cur, last, "radio track repeated immediately");
        last = cur;
    }

    // Switch station on the same emitter.
    assert!(player.tune(10, BuiltinStation::Disco, SpatialAttenuation::FULL));
    assert_eq!(
        player.emitter_station(10),
        Some(&StationId::Builtin(BuiltinStation::Disco))
    );
    assert!(player.emitter_track(10).unwrap().starts_with("disco_"));
    assert!(player.faults.is_empty(), "{:?}", player.faults);
}

#[test]
fn spatial_attenuation_pan_and_volume_steps_for_stereo_vs_tv() {
    let mut catalog = RadioCatalog::empty();
    catalog.insert(BuiltinStation::Rock, vec!["stereo.wav".into()]);
    catalog.insert(BuiltinStation::TvAction, vec!["tv.wav".into()]);

    let mut files = BTreeMap::new();
    files.insert("stereo.wav".into(), make_wav(RATE, 2, 2000, 12_000));
    files.insert("tv.wav".into(), make_wav(RATE, 2, 2000, 12_000));

    let mut player = RadioPlayer::new(catalog, files, 7);
    player.set_crossfade_ms(0);

    // Stereo hard left at full volume.
    assert!(player.tune(1, BuiltinStation::Rock, SpatialAttenuation::new(1024, 0)));
    let pcm = player.render_stereo(16);
    assert!(pcm[0] > 10_000, "left channel should be loud: {}", pcm[0]);
    assert_eq!(pcm[1], 0, "right channel should be silent at hard-left pan");

    // Distance & floor attenuation reduces amplitude monotonically.
    let near = SpatialAttenuation::from_distance(1.0, 10.0, true, 512);
    let far = SpatialAttenuation::from_distance(8.0, 10.0, true, 512);
    let other_floor = SpatialAttenuation::from_distance(1.0, 10.0, false, 512);
    assert!(near.volume > far.volume);
    assert!(near.volume > other_floor.volume);

    player.set_emitter_spatial(1, near);
    let near_sample = player.render_stereo(8)[0];
    player.set_emitter_spatial(1, far);
    let far_sample = player.render_stereo(8)[0];
    assert!(near_sample > far_sample);

    // Projection helper works identically to 1.91 placement.
    let view = Projection {
        at: [400, 300],
        screen: [800, 600],
        zoom: 3,
        shown_level: 0,
    };
    player.set_emitter_projection(1, 20, 0, &view).unwrap();
    assert_eq!(
        player.emitter_spatial(1),
        Some(SpatialAttenuation::new(1024, 512))
    );

    // Music volume step 0 mutes stereo, while TV (FX group) remains audible.
    player.set_music_volume_step(0);
    assert_eq!(player.music_volume(), VOLUME_STEPS[0]);
    let muted_stereo = player.render_stereo(8);
    assert!(muted_stereo.iter().all(|&s| s == 0));

    assert!(player.tune(2, BuiltinStation::TvAction, SpatialAttenuation::FULL));
    let tv_only = player.render_stereo(8);
    assert!(
        tv_only.iter().any(|&s| s > 0),
        "TV uses FX volume group, not Music"
    );

    player.set_fx_volume_step(0);
    assert_eq!(player.fx_volume(), VOLUME_STEPS[0]);
    let both_muted = player.render_stereo(8);
    assert!(both_muted.iter().all(|&s| s == 0));

    // Verify station kinds.
    assert_eq!(BuiltinStation::Rock.kind(), StationKind::Stereo);
    assert_eq!(BuiltinStation::TvAction.kind(), StationKind::Tv);
}

#[test]
fn unreadable_track_preserves_non_repeating_shuffle_and_crossfade_respects_muted_group() {
    // 1. Unreadable track in [missing, B, C] must not cause B, C, C.
    let mut catalog = RadioCatalog::empty();
    catalog.insert(
        BuiltinStation::Rock,
        vec!["missing.wav".into(), "b.wav".into(), "c.wav".into()],
    );
    catalog.insert(BuiltinStation::TvAction, vec!["tv.wav".into()]);
    let mut files = BTreeMap::new();
    files.insert("b.wav".into(), make_wav(RATE, 2, 120, 5_000));
    files.insert("c.wav".into(), make_wav(RATE, 2, 120, 6_000));
    files.insert("tv.wav".into(), make_wav(RATE, 2, 2000, 12_000));

    let mut player = RadioPlayer::new(catalog, files, 0);
    player.set_crossfade_ms(0);
    assert!(player.tune(1, BuiltinStation::Rock, SpatialAttenuation::FULL));
    let mut prev = player.emitter_track(1).unwrap().to_owned();
    for _ in 0..8 {
        let _ = player.render_stereo(125);
        let cur = player.emitter_track(1).unwrap().to_owned();
        assert_ne!(cur, prev, "track repeated immediately after missing entry");
        prev = cur;
    }

    // 2. Switching from muted TV (FX=0) to Rock (Music=10) with crossfade must
    //    not leak the muted TV deck into the fade.
    player.stop_emitter(1);
    player.set_crossfade_ms(100);
    player.set_fx_volume_step(0);
    player.set_music_volume_step(0);
    assert!(player.tune(2, BuiltinStation::TvAction, SpatialAttenuation::FULL));
    player.set_music_volume_step(10);
    // First sample of crossfade has incoming deck starting at gain 0, so any
    // non-zero sample would mean the muted TV leaked through.
    assert!(player.tune(2, BuiltinStation::Rock, SpatialAttenuation::FULL));
    let first = player.render_stereo(1);
    assert_eq!(first, vec![0, 0]);
}

#[test]
fn complete_collection_live_install_discovers_and_streams_radio_and_tv() {
    let Some(root) = install() else {
        return;
    };
    let files = crate::common::sound_files(&root);
    let catalog = RadioCatalog::from_sound_files(&files);

    // Every built-in stereo station in Complete Collection should have tracks.
    for station in BuiltinStation::STEREO {
        let id = StationId::Builtin(station);
        assert!(
            !catalog.tracks(&id).is_empty(),
            "missing tracks for built-in stereo station {station:?} ({})",
            station.directory()
        );
    }

    // Every built-in TV station in Complete Collection should have tracks.
    for station in BuiltinStation::TV {
        let id = StationId::Builtin(station);
        assert!(
            !catalog.tracks(&id).is_empty(),
            "missing tracks for built-in TV channel {station:?} ({})",
            station.directory()
        );
    }

    // Stream real MP3 stereo stations + real XA TV station simultaneously via
    // File-backed TrackSource without buffering entire MP3s into RAM.
    let mut player = RadioPlayer::new(catalog, files, 2026);
    assert!(player.tune(
        100,
        BuiltinStation::Classical,
        SpatialAttenuation::new(1024, 256)
    ));
    assert!(player.tune(
        200,
        BuiltinStation::TvAction,
        SpatialAttenuation::new(900, 768)
    ));

    let pcm = player.render_stereo(RATE as usize / 2); // 0.5 s
    let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    assert!(peak > 100, "expected audible radio+TV mix, peak={peak}");
    assert!(player.faults.is_empty(), "faults: {:?}", player.faults);

    // Switch Classical -> Rock and verify clean streaming.
    assert!(player.tune(100, BuiltinStation::Rock, SpatialAttenuation::FULL));
    let pcm_rock = player.render_stereo(RATE as usize / 4);
    let rock_peak = pcm_rock.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    assert!(
        rock_peak > 100,
        "expected audible Rock audio, peak={rock_peak}"
    );
    assert!(player.faults.is_empty(), "faults: {:?}", player.faults);
}
