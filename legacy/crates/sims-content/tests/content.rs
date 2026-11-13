use sims_content::Index;
use sims_formats::sprite::{Channel, render};
use std::{collections::BTreeSet, fs, path::PathBuf, time::SystemTime};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("sims-content-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn archive_and_loose_duplicates_keep_provenance_and_original_bytes() {
    let temp = Fixture::new();
    let loose = b"loose original";
    fs::create_dir(temp.0.join("Downloads")).unwrap();
    fs::write(temp.0.join("Downloads/chair.iff"), loose).unwrap();
    let payload = b"archived original";
    let mut archive = b"FAR!byAZ".to_vec();
    archive.extend(1u32.to_le_bytes());
    archive.extend((16 + payload.len() as u32).to_le_bytes());
    archive.extend(payload);
    archive.extend(1u32.to_le_bytes());
    for word in [payload.len() as u32, payload.len() as u32, 16, 9] {
        archive.extend(word.to_le_bytes());
    }
    archive.extend(b"Chair.iff");
    let source = temp.0.join("Objects.far");
    fs::write(&source, &archive).unwrap();

    let index = Index::scan(&temp.0).unwrap();
    assert!(index.diagnostics.is_empty());
    assert_eq!(index.archives, 1);
    assert_eq!(index.resources.len(), 2);
    assert!(
        index
            .find("chair.iff")
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    let archived = index.find("objects.FAR::chair.IFF").unwrap();
    assert_eq!(index.resources[archived].read().unwrap(), payload);
    let loose_id = index.find("downloads/CHAIR.IFF").unwrap();
    assert_eq!(index.resources[loose_id].read().unwrap(), loose);
    assert_eq!(fs::read(&source).unwrap(), archive);
    assert_eq!(fs::read(temp.0.join("Downloads/chair.iff")).unwrap(), loose);
    let mut colliding = index.clone();
    let mut duplicate = colliding.resources[archived].clone();
    duplicate.locator = duplicate.locator.to_ascii_uppercase();
    colliding.resources.push(duplicate);
    assert!(
        colliding
            .find("Objects.far::Chair.iff")
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    assert!(
        index
            .resources
            .windows(2)
            .all(|p| p[0].locator < p[1].locator)
    );
}

#[test]
#[ignore = "requires the user's private Complete Collection corpus; see README"]
fn original_chair_has_twelve_views_and_linked_sit_behavior() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let doc = index
        .document(index.find("DiningChairs.iff").unwrap())
        .unwrap();
    assert!(doc.diagnostics.is_empty());
    let chair = doc.objects.iter().find(|o| o.id == 16807).unwrap();
    assert_eq!(doc.object_title(chair), "Touch of Teak Dinette Chair");
    assert_eq!(chair.value.guid, 0xE7938987);
    let group = doc
        .groups
        .iter()
        .find(|g| g.id == chair.value.graphic)
        .unwrap();
    let actual: BTreeSet<_> = group
        .value
        .views
        .iter()
        .map(|v| (v.direction, v.zoom))
        .collect();
    let expected: BTreeSet<_> = [1, 4, 16, 64]
        .into_iter()
        .flat_map(|d| (1..=3).map(move |z| (d, z)))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(group.value.views.len(), 12);
    for view in &group.value.views {
        let preview = render(&doc.iff, view).unwrap();
        assert!(preview.notes.is_empty(), "{:?}", preview.notes);
        assert!(preview.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        assert!(preview.depth.iter().any(|d| *d < 255));
        for channel in [Channel::Color, Channel::Alpha, Channel::Depth] {
            assert_eq!(
                preview.channel(channel, false).len(),
                (preview.width * preview.height * 4) as usize
            );
        }
    }
    let menu = doc
        .tables
        .iter()
        .find(|t| t.id == chair.value.table)
        .unwrap();
    let sit = menu
        .value
        .interactions
        .iter()
        .find(|i| i.action == 0x1001)
        .unwrap();
    assert_eq!(sit.test, 0x1002);
    assert_eq!(
        doc.strings(b"TTAs", menu.id)
            .unwrap()
            .get(sit.string_index as usize, 1),
        Some("Sit")
    );
    for id in [sit.action, sit.test] {
        let routine = doc.behaviors.iter().find(|b| b.id == id).unwrap();
        assert!(!routine.value.instructions.is_empty());
        assert!(
            routine
                .value
                .disassemble(|id| doc.behavior_name(id))
                .contains("RETURN TRUE")
        );
    }
}
