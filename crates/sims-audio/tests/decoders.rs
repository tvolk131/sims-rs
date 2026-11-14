//! The decoders against synthetic files. No game data.

use sims_audio::decode::Kind;
use sims_audio::files::{Entry, SoundFiles, Source, key};
use sims_audio::{Pcm, mp3, wav, xa};

fn xa_file(channels: u16, rate: u32, decoded_bytes: u32, body: &[u8]) -> Vec<u8> {
    let mut out = b"XA\0\0".to_vec();
    out.extend(decoded_bytes.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2 * u32::from(channels)).to_le_bytes());
    out.extend((2 * channels).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(body);
    out
}

#[test]
fn xa_with_no_predictor_and_shift_twelve_returns_the_nibbles() {
    // Predictor 0 adds nothing; shift 12 scales a nibble by 2^8, and the
    // rounding division by 256 takes it back.
    let mut body = vec![0x0c, 0x78, 0xf1];
    body.extend([0; 12]);
    let pcm = xa::decode(&xa_file(1, 22050, 8, &body)).unwrap();
    assert_eq!(pcm.rate, 22050);
    assert_eq!(pcm.channels, 1);
    assert_eq!(pcm.samples, [7, -8, -1, 1]);
}

#[test]
fn xa_predictors_weigh_the_two_previous_outputs() {
    // A low nibble of 0 in the frame header shifts by 20: one step is 4096
    // after the division. Predictor 1 adds 240/256 of the last output;
    // predictor 2 adds 460/256 of it and -208/256 of the one before.
    let mut body = vec![0x10, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    body.extend([0x20, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let pcm = xa::decode(&xa_file(1, 11025, 4 * 28 * 2 / 2, &body)).unwrap();
    // Frame 1: 0, 4096, then 4096 decays by 240/256 per sample...
    let mut want = vec![0i32, 4096];
    let (mut current, mut previous) = (4096i32, 0i32);
    for _ in 2..28 {
        let next = (current * 240 + 128) >> 8;
        previous = current;
        current = next;
        want.push(next);
    }
    // ...and frame 2 starts from the last two outputs with predictor 2.
    for _ in 0..28 {
        let next = (current * 460 + previous * -208 + 128) >> 8;
        previous = current;
        current = next;
        want.push(next);
    }
    let want: Vec<i16> = want.into_iter().map(|s| s as i16).collect();
    assert_eq!(pcm.samples, want);
}

#[test]
fn xa_clamps_to_sixteen_bits() {
    // Shift 20, nibble 7: 7 * 4096 = 28672, then predictor 1 pushes past
    // 32767; nibble 8 (-8) pulls below -32768.
    let mut body = vec![0x10, 0x77, 0x77];
    body.extend([0x88; 12]);
    let pcm = xa::decode(&xa_file(1, 22050, 56, &body)).unwrap();
    assert_eq!(&pcm.samples[..4], [28672, 32767, 32767, 32767]);
    assert!(pcm.samples.contains(&i16::MIN));
}

#[test]
fn xa_stereo_alternates_left_and_right_by_nibble() {
    // Left header, right header, then one byte per channel per group: the
    // high nibbles make the first stereo frame, the low nibbles the second.
    let mut body = vec![0x0c, 0x0c, 0x12, 0x34];
    body.extend([0; 26]);
    let pcm = xa::decode(&xa_file(2, 22050, 8, &body)).unwrap();
    assert_eq!(pcm.channels, 2);
    assert_eq!(pcm.samples, [1, 3, 2, 4]);
}

#[test]
fn xa_decodes_a_partial_last_frame_and_cuts_to_the_decoded_size() {
    // One full frame, then a frame with its header and two of its groups.
    let mut body = vec![0x0c];
    body.extend([0x11; 14]);
    body.extend([0x0c, 0x23, 0x45]);
    let pcm = xa::decode(&xa_file(1, 22050, (28 + 3) * 2, &body)).unwrap();
    assert_eq!(pcm.samples.len(), 31);
    assert_eq!(&pcm.samples[27..], [1, 2, 3, 4]);
    // Asking for more than the data holds fails.
    let error = xa::decode(&xa_file(1, 22050, (28 + 5) * 2, &body)).unwrap_err();
    assert!(error.to_string().contains("32 of 33"), "{error}");
}

#[test]
fn xa_rejects_bad_headers() {
    let ok = xa_file(1, 22050, 0, &[]);
    assert!(xa::decode(&ok).unwrap().samples.is_empty());
    let mut not_pcm = ok.clone();
    not_pcm[8] = 2;
    assert!(xa::decode(&not_pcm).is_err());
    let mut three = ok.clone();
    three[10] = 3;
    assert!(xa::decode(&three).is_err());
    let mut odd = xa_file(2, 22050, 6, &[0; 30]);
    assert!(xa::decode(&odd).is_err());
    odd[4] = 8;
    assert!(xa::decode(&odd).is_ok());
    assert!(xa::decode(&ok[..20]).is_err());
    let mut bad_filter = xa_file(1, 22050, 56, &[0; 15]);
    bad_filter[24] = 0x40;
    assert!(xa::decode(&bad_filter).is_err());
}

/// An XA encoder written apart from the decoder: per frame it tries every
/// predictor and shift and keeps the closest. Decoding its output must
/// give back what it predicted, and stay near the input.
fn encode(input: &[i16]) -> (Vec<u8>, Vec<i16>) {
    const FILTERS: [(i64, i64); 4] = [(0, 0), (240, 0), (460, -208), (392, -220)];
    let mut body = Vec::new();
    let mut expected = Vec::new();
    let (mut cur, mut prev) = (0i64, 0i64);
    for frame in input.chunks(28) {
        // Error, header byte, nibbles, outputs, and the state after.
        type Trial = (i64, u8, Vec<u8>, Vec<i16>, i64, i64);
        let mut best: Option<Trial> = None;
        for filter in 0..4u8 {
            for shift in 0..16u8 {
                let (w0, w1) = FILTERS[usize::from(filter)];
                let (mut c, mut p) = (cur, prev);
                let mut nibbles = Vec::new();
                let mut out = Vec::new();
                let mut error = 0;
                for i in 0..28 {
                    let want = i64::from(frame.get(i).copied().unwrap_or(0));
                    let mut pick = (0u8, 0i64, i64::MAX);
                    for n in -8i64..8 {
                        let v = ((n << (20 - shift)) + c * w0 + p * w1 + 128) >> 8;
                        let v = v.clamp(-32768, 32767);
                        if (v - want).abs() < pick.2 {
                            pick = ((n & 15) as u8, v, (v - want).abs());
                        }
                    }
                    p = c;
                    c = pick.1;
                    nibbles.push(pick.0);
                    out.push(pick.1 as i16);
                    error += pick.2;
                }
                if best.as_ref().is_none_or(|b| error < b.0) {
                    best = Some((error, filter << 4 | shift, nibbles, out, c, p));
                }
            }
        }
        let (_, header, nibbles, out, c, p) = best.unwrap();
        body.push(header);
        body.extend(nibbles.chunks(2).map(|n| n[0] << 4 | n[1]));
        expected.extend(&out[..frame.len()]);
        (cur, prev) = (c, p);
    }
    (body, expected)
}

#[test]
fn xa_inverts_an_independent_encoder() {
    // A triangle wave and a burst, in integers.
    let input: Vec<i16> = (0..2000)
        .map(|i: i32| {
            let tri = (i % 200 - 100).abs() * 300 - 15000;
            let burst = if (700..760).contains(&i) { 9000 } else { 0 };
            (tri + burst) as i16
        })
        .collect();
    let (body, expected) = encode(&input);
    let pcm = xa::decode(&xa_file(1, 22050, input.len() as u32 * 2, &body)).unwrap();
    assert_eq!(pcm.samples, expected);
    // A greedy encoder overshoots at the burst's edges, so bound the mean.
    let total: i64 = input
        .iter()
        .zip(&pcm.samples)
        .map(|(a, b)| (i64::from(*a) - i64::from(*b)).abs())
        .sum();
    let mean = total / input.len() as i64;
    assert!(mean < 300, "mean error {mean}");
}

fn wav_file(channels: u16, bits: u16, extra: &[(&[u8; 4], &[u8])], data: &[u8]) -> Vec<u8> {
    let mut chunks = Vec::new();
    let mut chunk = |id: &[u8; 4], body: &[u8]| {
        chunks.extend(id);
        chunks.extend((body.len() as u32).to_le_bytes());
        chunks.extend(body);
        if body.len() % 2 == 1 {
            chunks.push(0);
        }
    };
    let block = channels * bits / 8;
    let mut fmt = Vec::new();
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(channels.to_le_bytes());
    fmt.extend(22050u32.to_le_bytes());
    fmt.extend((22050 * u32::from(block)).to_le_bytes());
    fmt.extend(block.to_le_bytes());
    fmt.extend(bits.to_le_bytes());
    chunk(b"fmt ", &fmt);
    for (id, body) in extra {
        chunk(id, body);
    }
    chunk(b"data", data);
    let mut out = b"RIFF".to_vec();
    out.extend((4 + chunks.len() as u32).to_le_bytes());
    out.extend(b"WAVE");
    out.extend(chunks);
    out
}

#[test]
fn wav_reads_pcm_and_skips_padded_chunks() {
    let data: Vec<u8> = [1i16, -2, 300, -32768]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let bytes = wav_file(2, 16, &[(b"LIST", b"odd")], &data);
    let pcm = wav::decode(&bytes).unwrap();
    assert_eq!(pcm.channels, 2);
    assert_eq!(pcm.rate, 22050);
    assert_eq!(pcm.samples, [1, -2, 300, -32768]);
    assert_eq!(pcm.frames(), 2);
    assert_eq!(wav::format(&bytes).unwrap().bits, 16);
    // Our own writer reads back.
    assert_eq!(wav::decode(&pcm.to_wav()).unwrap(), pcm);
}

#[test]
fn wav_reads_eight_bit_and_drops_partial_frames() {
    let pcm = wav::decode(&wav_file(1, 8, &[], &[128, 255, 0])).unwrap();
    assert_eq!(pcm.samples, [0, 127 << 8, -128 << 8]);
    let pcm = wav::decode(&wav_file(2, 16, &[], &[1, 0, 2, 0, 3, 0])).unwrap();
    assert_eq!(pcm.samples, [1, 2]);
}

#[test]
fn wav_rejects_what_it_cannot_read() {
    assert!(wav::decode(b"RIFF\0\0\0\0WAVE").is_err());
    let mut adpcm = wav_file(1, 16, &[], &[0, 0]);
    adpcm[20] = 2;
    assert!(wav::decode(&adpcm).is_err());
    let data_first = b"RIFF\x0c\0\0\0WAVEdata\0\0\0\0";
    assert!(wav::decode(data_first).is_err());
}

/// MPEG-1 Layer III frames whose side information is all zero: they
/// decode to silence. 128 kbit/s at 44,100 Hz makes 417-byte frames.
fn silent_mp3(frames: usize, mono: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..frames {
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, if mono { 0xc0 } else { 0x00 }]);
        out.extend(frame);
    }
    out
}

#[test]
fn mp3_scans_and_decodes_silent_frames() {
    for (mono, channels) in [(false, 2), (true, 1)] {
        let bytes = silent_mp3(5, mono);
        assert_eq!(Kind::sniff(&bytes), Some(Kind::Mp3));
        let info = mp3::scan(&bytes).unwrap();
        assert_eq!(info.rate, 44100);
        assert_eq!(info.channels, channels);
        assert_eq!(info.packets, 5);
        assert_eq!(info.frames, 5 * 1152);
        let pcm = mp3::decode(&bytes).unwrap();
        assert_eq!(pcm.channels, channels);
        assert_eq!(pcm.rate, 44100);
        assert_eq!(pcm.frames(), 5 * 1152);
        assert!(pcm.samples.iter().all(|&s| s == 0));
        assert_eq!(mp3::decode_packets(&bytes, 2).unwrap().frames(), 2 * 1152);
    }
}

#[test]
fn kinds_are_sniffed_from_magic_numbers() {
    assert_eq!(Kind::sniff(b"XAJ\0"), Some(Kind::Xa));
    assert_eq!(Kind::sniff(b"RIFF\0\0\0\0WAVE"), Some(Kind::Wav));
    assert_eq!(Kind::sniff(b"ID3\x03"), Some(Kind::Mp3));
    assert_eq!(Kind::sniff(b"RIFF\0\0\0\0AVI "), None);
    assert_eq!(Kind::sniff(b"HIT!"), None);
    let pcm = Pcm {
        rate: 22050,
        channels: 1,
        samples: vec![5, -5],
    };
    assert_eq!(
        sims_audio::decode::decode(&pcm.to_wav()).unwrap().unwrap(),
        pcm
    );
}

#[test]
fn sound_paths_compare_without_case_or_separator() {
    assert_eq!(
        key("SoundData/Phone\\Phone_Ring.XA"),
        "sounddata\\phone\\phone_ring.xa"
    );
    assert_eq!(key(".\\\\a\\B"), "a\\b");
}

#[test]
fn sound_files_mount_archives_in_their_directory() {
    let root = std::env::temp_dir().join(format!("sims-audio-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("SoundData")).unwrap();
    // An archive with a member in a subdirectory, a duplicate, and a
    // member a loose file replaces.
    let members: [(&str, &[u8]); 4] = [
        ("Phone\\Ring.XA", b"one"),
        ("Phone\\Ring.XA", b"two"),
        ("loose.hit", b"member"),
        ("picture.bmp", b"skip"),
    ];
    let mut far = b"FAR!byAZ".to_vec();
    far.extend(1u32.to_le_bytes());
    far.extend(0u32.to_le_bytes());
    let mut offsets = Vec::new();
    for (_, data) in members {
        offsets.push(far.len() as u32);
        far.extend(data);
    }
    let directory = far.len() as u32;
    far[12..16].copy_from_slice(&directory.to_le_bytes());
    far.extend((members.len() as u32).to_le_bytes());
    for ((name, data), offset) in members.iter().zip(offsets) {
        far.extend((data.len() as u32).to_le_bytes());
        far.extend((data.len() as u32).to_le_bytes());
        far.extend(offset.to_le_bytes());
        far.extend((name.len() as u32).to_le_bytes());
        far.extend(name.as_bytes());
    }
    std::fs::write(root.join("SoundData/SoundData.far"), far).unwrap();
    std::fs::write(root.join("SoundData/loose.hit"), b"loose").unwrap();
    let files = SoundFiles::scan(&root).unwrap();
    assert_eq!(files.len(), 2);
    let ring = files.get("sounddata/phone/ring.xa").unwrap();
    assert_eq!(ring.read().unwrap(), b"one");
    assert_eq!(ring.source.read_head(2).unwrap(), b"on");
    assert_eq!(ring.extension(), "xa");
    let hit = files.get("SOUNDDATA\\LOOSE.HIT").unwrap();
    assert!(matches!(hit.source, Source::Loose(_)));
    assert_eq!(hit.read().unwrap(), b"loose");
    let shadowed: Vec<_> = files.shadowed.iter().map(Entry::read).collect();
    assert_eq!(shadowed.len(), 2);
    assert!(files.problems.is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}
