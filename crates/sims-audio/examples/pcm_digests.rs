//! Prints `path<TAB>frames<TAB>sha256` of the decoded PCM of every XA and
//! WAV file in an install, for comparison with another decoder.
use sha2::{Digest, Sha256};
use sims_audio::files::SoundFiles;

fn main() {
    let root = std::env::args().nth(1).expect("install path");
    let files = SoundFiles::scan(root.as_ref()).unwrap();
    for entry in files.iter() {
        if !["xa", "wav"].contains(&entry.extension().as_str()) {
            continue;
        }
        let pcm = sims_audio::decode::decode(&entry.read().unwrap())
            .unwrap()
            .unwrap();
        let digest: String = Sha256::digest(pcm.bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        println!("{}\t{}\t{}", entry.path, pcm.samples.len(), digest);
    }
}
