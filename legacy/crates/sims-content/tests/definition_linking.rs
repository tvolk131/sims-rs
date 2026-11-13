//! Registered definition owners differ from immutable raw OBJD chunks. Keep
//! native normalization and exact historical relinking as separate boundaries.
use sims_content::{Index, vm};
use sims_vm::{Program, Snapshot, Vm, World};
use std::{collections::BTreeSet, fs, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sims-definition-link-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn iff(data: Option<Vec<u8>>) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    if let Some(data) = data {
        bytes.extend(b"OBJD");
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(128u16.to_be_bytes());
        bytes.extend([0; 66]);
        bytes.extend(data);
    }
    bytes
}
fn profile(p: &Program) -> BTreeSet<String> {
    p.modules
        .iter()
        .filter(|(_, m)| m.normalized_object_definitions)
        .map(|(name, _)| name.clone())
        .collect()
}
#[test]
fn native_link_normalizes_resident_words_without_changing_raw_asset_or_hash() {
    for (version, length) in [
        (136u32, 160usize),
        (137, 161),
        (138, 216),
        (139, 220),
        (140, 211),
    ] {
        let root = Fixture::new();
        let mut raw = vec![0; length];
        raw[..4].copy_from_slice(&version.to_le_bytes());
        if length & 1 != 0 {
            raw[length - 1] = 0x7b;
        }
        let bytes = iff(Some(raw.clone()));
        fs::write(root.0.join("Cursor.iff"), &bytes).unwrap();
        fs::write(root.0.join("Global.iff"), iff(None)).unwrap();
        let index = Index::scan(&root.0).unwrap();
        let mut p = vm::load_program(&index, "Cursor.iff", "Global.iff").unwrap();
        let mut expected = raw.clone();
        sims_sim::object_catalog::definition::normalize(&mut expected).unwrap();
        let actual: Vec<u8> = p.modules["Cursor.iff"].object_definition_words[&128]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        assert_eq!(actual, expected, "version{version} len{length}");
        assert_eq!(fs::read(root.0.join("Cursor.iff")).unwrap(), bytes);
        use sha2::{Digest, Sha256};
        assert_eq!(
            p.modules["Cursor.iff"].content_hash,
            format!("{:x}", Sha256::digest(&bytes))
        );
        let native = p.fingerprint();
        let provenance = profile(&p);
        p.modules
            .get_mut("Cursor.iff")
            .unwrap()
            .normalized_object_definitions = false;
        assert_eq!(
            p.fingerprint(),
            native,
            "marker must not change historical fingerprint schema"
        );
        vm::restore_definition_profiles(&index, &mut p, &provenance).unwrap();
        assert_eq!(p.fingerprint(), native);
        vm::restore_definition_profiles(&index, &mut p, &BTreeSet::new()).unwrap();
        assert!(profile(&p).is_empty());
        assert_eq!(
            p.modules["Cursor.iff"].object_definition_words[&128],
            raw.chunks_exact(2)
                .take(108)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>()
        );
        let legacy = p.fingerprint();
        let snapshot = Snapshot::capture(&p, &Vm::new(World::default(), 0x1234));
        snapshot.clone().restore(&p).unwrap();
        vm::restore_definition_profiles(&index, &mut p, &provenance).unwrap();
        if legacy != native {
            assert!(snapshot.restore(&p).unwrap_err().contains("program"));
        }
        let before = serde_json::to_value(&p).unwrap();
        assert!(
            vm::restore_definition_profiles(
                &index,
                &mut p,
                &BTreeSet::from(["Missing.iff".into()])
            )
            .is_err()
        );
        assert_eq!(serde_json::to_value(&p).unwrap(), before);
        fs::write(root.0.join("Cursor.iff"), iff(Some(vec![0; length]))).unwrap();
        assert!(vm::restore_definition_profiles(&index, &mut p, &BTreeSet::new()).is_err());
        assert_eq!(serde_json::to_value(&p).unwrap(), before);
    }
}

#[test]
#[ignore = "private immutable original assets; actual Cursors128 native registered definition"]
fn original_cursor128_is_normalized_and_legacy_relink_remains_exact() -> anyhow::Result<()> {
    let index = Index::scan(&PathBuf::from(std::env::var("SIMS_DEFINITION_DATA_ROOT")?))?;
    let source = index.resources[index.find("Cursors.iff")?].locator.clone();
    let raw = index.resources[index.find(&source)?].read()?;
    let iff = sims_formats::iff::Iff::parse(raw.clone())?;
    let chunk = iff.get(b"OBJD", 128).unwrap();
    assert_eq!(iff.data(chunk).len(), 160);
    assert_eq!(&iff.data(chunk)[..4], &136u32.to_le_bytes());
    let mut p = vm::load_program(&index, &source, "Global.iff")?;
    let words = &p.modules[&source].object_definition_words[&128];
    assert_eq!(words.len(), 108);
    assert_eq!(words[0], 138);
    assert_eq!(words[102], 0);
    let normalized = profile(&p);
    let native = p.fingerprint();
    vm::restore_definition_profiles(&index, &mut p, &BTreeSet::new())?;
    assert_eq!(p.modules[&source].object_definition_words[&128].len(), 80);
    assert_eq!(p.modules[&source].object_definition_words[&128][0], 136);
    assert_ne!(p.fingerprint(), native);
    vm::restore_definition_profiles(&index, &mut p, &normalized)?;
    assert_eq!(p.fingerprint(), native);
    assert_eq!(index.resources[index.find(&source)?].read()?, raw);
    Ok(())
}

#[test]
#[ignore = "private real Buy-preview checkpoint: mixed historical/native definition provenance"]
fn actual_buy_cursor_retains_native_definition_across_managed_encode_decode() -> anyhow::Result<()>
{
    std::thread::Builder::new().stack_size(64*1024*1024).spawn(|| -> anyhow::Result<()> {
        use anyhow::{Context, ensure};
        use sims_runtime::original_session::{Saved, Session};
        let index = Index::scan(&PathBuf::from(std::env::var("SIMS_DEFINITION_DATA_ROOT")?))?;
        let bytes = fs::read(std::env::var("SIMS_DEFINITION_CHECKPOINT")?)?;
        let checkpoint: serde_json::Value = serde_json::from_slice(&bytes)?;
        let saved: Saved = serde_json::from_value(checkpoint["session"].clone())?;
        let session = Session::restore(&index, saved.clone())?;
        let vm = &session.simulation.vm;
        let definition = vm.world.objects.get(&161).context("actual cursor161 absent")?
            .definition.as_ref().context("actual cursor161 definition absent")?;
        ensure!(definition.guid ==1079 && definition.object ==128);
        let module = &session.program.modules[&definition.source];
        ensure!(module.normalized_object_definitions && saved.normalized_object_definitions.contains(&definition.source));
        let words = &module.object_definition_words[&128];
        ensure!(words.len()==108 && words[0]==138 && words[102]==0);
        ensure!(session.program.modules.values().any(|m|!m.normalized_object_definitions),
            "fixture must retain historical modules alongside new native links");
        let readiness = session.host.ordinary.presenter_readiness(vm, &session.program)?;
        ensure!(readiness[&161]==0);
        let encoded = serde_json::to_vec(&session.save_managed()?)?;
        let roundtrip: Saved = serde_json::from_slice(&encoded)?;
        ensure!(roundtrip.normalized_object_definitions ==saved.normalized_object_definitions);
        let cold = Session::restore(&index, roundtrip)?;
        ensure!(serde_json::to_value(session.save())?==serde_json::to_value(cold.save())?,
            "mixed native/raw managed encode/decode changed state");
        ensure!(cold.program.modules[&definition.source].normalized_object_definitions);
        let mut forged = session.save_managed()?;
        forged.normalized_object_definitions.remove(&definition.source);
        let mut wrong_program = session.program.clone();
        vm::restore_definition_profiles(session.index(), &mut wrong_program,
            &forged.normalized_object_definitions)?;
        ensure!(wrong_program.fingerprint() != forged.simulation.program_hash,
            "raw cursor profile unexpectedly retained native program hash");
        drop(wrong_program);
        let error = Session::restore(&index, forged).err().context("raw cursor forgery accepted")?;
        ensure!(error.to_string() == "room snapshot version/content mismatch",
            "forged-profile rejection differs: {error:#}");
        println!("cursor161 source={} GUID={} words={} version={} flags={} readiness={} tick={} historical_modules={} native_modules={} exact_mixed_cold=true raw_forgery_rejected={error:#}",
            definition.source, definition.guid,words.len(),words[0],words[102],readiness[&161],vm.tick,
            session.program.modules.values().filter(|m|!m.normalized_object_definitions).count(),
            saved.normalized_object_definitions.len());
        Ok(())
    })?.join().expect("mixed definition proof panicked")
}
