use anyhow::{Result, ensure};
use sims_content::{Index, Resource, graphic_profiles::Cached};
use sims_vm::Definition;

fn definition() -> Definition {
    Definition {
        source: "fixture/Profile.iff".into(),
        object: 1,
        guid: 1,
        slots: 0,
    }
}
fn words(kind: i16) -> Vec<i16> {
    let mut words = vec![0; 20];
    words[9] = kind;
    words[19] = 200;
    words
}
fn empty_index() -> Index {
    Index {
        root: Default::default(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    }
}

#[test]
fn person_profile_does_not_open_an_ordinary_resource() -> Result<()> {
    let index = empty_index();
    let mut cache = Cached::new(definition(), &words(2))?;
    ensure!(cache.advance(&index, &mut |_, _, _| panic!("unexpected writeback"))?);
    ensure!(cache.strings()?.is_none());
    let saved = Cached::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&cache)?)?,
    )?;
    ensure!(saved == cache);
    Ok(())
}

#[test]
fn short_source_profile_exposes_writeback_preserves_cache_and_checks_saved_source() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("sims-ordinary-profile-{}", std::process::id()));
    std::fs::create_dir(&directory)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let path = directory.join("Profile.iff");
    // One English string, version -1. Native ObjectStrings must append the
    // missing second entry and request writeback, retaining its read result.
    let payload = [
        (-1_i16).to_le_bytes().as_slice(),
        1_u16.to_le_bytes().as_slice(),
        b"root\0",
    ]
    .concat();
    let mut bytes = b"IFF FILE 2.5:".to_vec();
    bytes.resize(64, 0);
    bytes.extend(b"STR#");
    bytes.extend(((76 + payload.len()) as u32).to_be_bytes());
    bytes.extend(200_u16.to_be_bytes());
    bytes.extend(0_u16.to_be_bytes());
    bytes.extend([0; 64]);
    bytes.extend(payload);
    std::fs::write(&path, &bytes)?;
    let mut index = empty_index();
    index.resources.push(Resource {
        locator: definition().source,
        name: "Profile.iff".into(),
        source: path.clone(),
        offset: 0,
        size: bytes.len() as u64,
        unpacked_size: bytes.len() as u64,
        archived: false,
        owned: None,
    });
    let mut cache = Cached::new(definition(), &words(4))?;
    ensure!(!cache.advance(&index, &mut |_, _, _| panic!("write before Load"))?);
    let saved = cache.clone();
    cache = Cached::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&cache)?)?,
    )?;
    ensure!(cache == saved && cache.strings().is_err());
    let mut writes = vec![];
    while !cache.complete() {
        cache.advance(&index, &mut |definition, id, strings| {
            writes.push((definition.clone(), id, strings.to_vec()));
            Ok(0) // Explicit backend failure; original load ignores this value.
        })?;
    }
    ensure!(writes == vec![(definition(), 200, vec!["root".to_string(), String::new()])]);
    ensure!(cache.strings()? == Some(["root".to_string(), String::new()].as_slice()));
    ensure!(std::fs::read(&path)? == bytes, "source was modified");
    let saved: Cached = serde_json::from_slice(&serde_json::to_vec(&cache)?)?;
    ensure!(Cached::restore(&index, saved.clone())? == cache);
    let warm = cache.clone();
    ensure!(
        cache.advance(&empty_index(), &mut |_, _, _| panic!("write on cache hit"))?
            && warm == cache
    );
    *bytes.last_mut().unwrap() = 1;
    std::fs::write(&path, bytes)?;
    ensure!(
        Cached::restore(&index, saved)
            .unwrap_err()
            .to_string()
            .contains("source changed")
    );
    Ok(())
}
