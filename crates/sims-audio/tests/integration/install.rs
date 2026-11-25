//! Every sound file of a real install decodes. Needs `SIMS_INSTALL`; skips
//! without it. Pins counts and digests of decoded PCM, no game data.

use sha2::{Digest, Sha256};
use sims_audio::decode::Kind;
use sims_audio::files::{self, key};
use sims_audio::{mp3, xa};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    crate::common::install("the install sound census")
}

/// FNV-1a over 16-bit samples: cheap enough for a billion samples in an
/// unoptimized build, where SHA-256 is not.
fn fnv(hash: &mut u64, samples: &[i16]) {
    for &s in samples {
        *hash = (*hash ^ u64::from(s as u16)).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// MPEG frames of each MP3 file decoded in the census: about 1.3 seconds.
/// The whole files decode in `every_mp3_decodes_in_full`, which is slow in
/// a debug build and so is ignored by default.
const MP3_PACKETS: u64 = 50;

#[test]
fn no_file_outside_the_sound_extensions_holds_sound() {
    let Some(root) = install() else { return };
    let (entries, problems) = files::walk(&root, &|_| true).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let mut census: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for entry in &entries {
        let head = entry.source.read_head(12).unwrap();
        let extension = entry.extension();
        let magic = if head.starts_with(b"XA") {
            "XA"
        } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
            "WAVE"
        } else if head.starts_with(b"UTM0") {
            "UTM0"
        } else if head.starts_with(b"ID3") {
            "ID3"
        } else if head.len() >= 2 && head[0] == 0xff && head[1] & 0xe0 == 0xe0 {
            "MPEG sync"
        } else {
            continue;
        };
        *census.entry((extension, magic)).or_default() += 1;
    }
    let census: Vec<_> = census
        .into_iter()
        .map(|((e, m), n)| format!("{e} {m} {n}"))
        .collect();
    // Every XA, WAVE and MPEG file carries its own extension. The `.cfp`
    // files (animation curves) are the only others to start with an MPEG
    // frame sync, by chance of their first value. No UTK anywhere.
    assert_eq!(
        census,
        [
            "cfp MPEG sync 740",
            "mp3 MPEG sync 141",
            "wav WAVE 285",
            "xa XA 18155"
        ]
    );
}

/// What decoding one file found.
struct Decoded {
    kind: &'static str,
    /// The format's variant: rate, channels, and the XA header's tag.
    variant: String,
    frames: u64,
    /// Frames the MP3 frame headers count, for MP3 files.
    scanned: u64,
    /// FNV-1a over the path, rate, channels and samples.
    hash: u64,
}

fn decode_one(entry: &files::Entry) -> Result<Decoded, String> {
    let bytes = entry.read().map_err(|e| e.to_string())?;
    let fail = |e: sims_audio::Error| format!("{}: {e}", entry.path);
    let (kind, variant, scanned, pcm) = match Kind::sniff(&bytes) {
        Some(Kind::Mp3) => {
            let info = mp3::scan(&bytes).map_err(fail)?;
            let variant = format!("{} Hz {} ch", info.rate, info.channels);
            let pcm = mp3::decode_packets(&bytes, MP3_PACKETS).map_err(fail)?;
            (Kind::Mp3, variant, info.frames, pcm)
        }
        Some(Kind::Xa) => {
            let header = xa::Header::parse(&bytes).map_err(fail)?;
            let variant = format!(
                "tag {:02x}{:02x} {} Hz {} ch",
                header.tag[0], header.tag[1], header.rate, header.channels
            );
            (Kind::Xa, variant, 0, xa::decode(&bytes).map_err(fail)?)
        }
        Some(Kind::Wav) => {
            let pcm = Kind::Wav.decode(&bytes).map_err(fail)?;
            let variant = format!("{} Hz {} ch", pcm.rate, pcm.channels);
            (Kind::Wav, variant, 0, pcm)
        }
        None => return Err(format!("{}: not a sound file", entry.path)),
    };
    let mut hash = 0xcbf2_9ce4_8422_2325;
    let path: Vec<i16> = key(&entry.path).bytes().map(i16::from).collect();
    fnv(&mut hash, &path);
    fnv(
        &mut hash,
        &[
            pcm.rate as i16,
            (pcm.rate >> 16) as i16,
            pcm.channels as i16,
        ],
    );
    fnv(&mut hash, &pcm.samples);
    Ok(Decoded {
        kind: kind.name(),
        variant,
        frames: pcm.frames() as u64,
        scanned,
        hash,
    })
}

#[test]
fn every_sound_file_in_the_install_decodes() {
    let Some(root) = install() else { return };
    let files = crate::common::sound_files(&root);
    assert!(files.problems.is_empty(), "{:?}", files.problems);
    // Every entry, shadowed ones too, so every file on disk is decoded.
    let all: Vec<&files::Entry> = files.iter().chain(&files.shadowed).collect();
    let mut by_extension: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &all {
        *by_extension.entry(entry.extension()).or_default() += 1;
    }
    let sound: Vec<&files::Entry> = all
        .into_iter()
        .filter(|e| ["xa", "wav", "mp3", "utk"].contains(&e.extension().as_str()))
        .collect();
    // An unoptimized build decodes about two billion samples here, so the
    // work is split over threads; results are combined in entry order.
    let workers = 6;
    let chunk = sound.len().div_ceil(workers).max(1);
    let results: Vec<Result<Decoded, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = sound
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(|e| decode_one(e)).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });
    let mut variants: BTreeMap<String, usize> = BTreeMap::new();
    let mut frames: BTreeMap<String, u64> = BTreeMap::new();
    let mut digests: BTreeMap<&str, u64> = BTreeMap::new();
    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok(d) => {
                *variants
                    .entry(format!("{} {}", d.kind, d.variant))
                    .or_default() += 1;
                *frames.entry(d.kind.to_owned()).or_default() += d.frames;
                if d.scanned > 0 {
                    *frames.entry(format!("{} (headers)", d.kind)).or_default() += d.scanned;
                }
                let digest = digests.entry(d.kind).or_insert(0xcbf2_9ce4_8422_2325);
                fnv(
                    digest,
                    &[
                        d.hash as i16,
                        (d.hash >> 16) as i16,
                        (d.hash >> 32) as i16,
                        (d.hash >> 48) as i16,
                    ],
                );
            }
            Err(error) => failures.push(error),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    let digests: BTreeMap<_, _> = digests
        .into_iter()
        .map(|(k, d)| (k, format!("{d:016x}")))
        .collect();
    eprintln!("{by_extension:?}\n{variants:?}\n{frames:?}\n{digests:?}");
    assert_eq!((files.len(), files.shadowed.len()), (18_426, 321));
    assert_eq!(
        format!("{by_extension:?}"),
        r#"{"fsc": 4, "hit": 54, "hot": 54, "hsm": 54, "mp3": 141, "wav": 285, "xa": 18155}"#
    );
    assert_eq!(
        format!("{variants:?}"),
        concat!(
            r#"{"MP3 44100 Hz 2 ch": 141, "WAV 22050 Hz 1 ch": 44, "WAV 22050 Hz 2 ch": 241, "#,
            r#""XA tag 0000 11025 Hz 1 ch": 118, "XA tag 0000 22050 Hz 1 ch": 9350, "#,
            r#""XA tag 0000 22050 Hz 2 ch": 2, "XA tag 1200 11025 Hz 1 ch": 114, "#,
            r#""XA tag 1200 22050 Hz 1 ch": 6440, "XA tag 4a00 11025 Hz 1 ch": 248, "#,
            r#""XA tag 4a00 22050 Hz 1 ch": 1883}"#
        )
    );
    assert_eq!(
        format!("{frames:?}"),
        r#"{"MP3": 8121600, "MP3 (headers)": 1359853056, "WAV": 68390909, "XA": 1934036109}"#
    );
    // MP3 PCM comes from floating-point decoding; its digest is pinned on
    // the machine that took it (Apple Silicon, macOS).
    assert_eq!(
        format!("{digests:?}"),
        r#"{"MP3": "f1f6ff1f48aadff8", "WAV": "acc775684f166799", "XA": "43cb6ad6221535fc"}"#
    );
}

#[test]
fn known_samples_decode_to_pinned_pcm() {
    let Some(root) = install() else { return };
    let files = crate::common::sound_files(&root);
    let mut got_all = Vec::new();
    for path in [
        "sounddata\\phone\\Phone_ring.xa",
        "sounddata\\footstep\\footstep_soft1.xa",
        "Music\\Modes\\Load\\loadloop.wav",
    ] {
        let entry = files
            .get(path)
            .unwrap_or_else(|| panic!("{path} not found"));
        let pcm = sims_audio::decode::decode(&entry.read().unwrap())
            .unwrap()
            .unwrap();
        let got = format!(
            "{} Hz {} ch {} frames {}",
            pcm.rate,
            pcm.channels,
            pcm.frames(),
            hex(&Sha256::digest(pcm.bytes()))
        );
        got_all.push(format!("{path}: {got}"));
    }
    assert_eq!(
        got_all,
        [
            "sounddata\\phone\\Phone_ring.xa: 22050 Hz 1 ch 57583 frames \
             22df7ed343bda0881d47d6e43465b509b692fb2b0a1f98d87cc823a0ccfb6c58",
            "sounddata\\footstep\\footstep_soft1.xa: 22050 Hz 1 ch 10863 frames \
             533c9bce5b9faaac10413e06c3ce534c3375756d7aae4387a1972b5c7fcbff27",
            "Music\\Modes\\Load\\loadloop.wav: 22050 Hz 2 ch 847501 frames \
             a8296e919ad936ea4082548633ce1fa785a99adc00740b7cfd9ad349fc5707a1",
        ]
    );
}

#[test]
#[ignore = "decodes about eight hours of music; run with --release -- --ignored"]
fn every_mp3_decodes_in_full() {
    let Some(root) = install() else { return };
    let files = crate::common::sound_files(&root);
    let mut count = 0;
    for entry in files.iter().filter(|e| e.extension() == "mp3") {
        let bytes = entry.read().unwrap();
        let info = mp3::scan(&bytes).unwrap();
        let pcm = mp3::decode(&bytes).unwrap_or_else(|e| panic!("{}: {e}", entry.path));
        assert_eq!(pcm.frames() as u64, info.frames, "{}", entry.path);
        count += 1;
    }
    assert_eq!(count, 141);
}
