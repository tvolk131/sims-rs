//! Pets, which the original draws as people: their skeletons load, and
//! the person type's body strings bind all their attach points. Needs
//! `SIMS_INSTALL`; skipped when it is unset, or on an install without
//! the pack that brings pets. Nothing from the game is
//! kept: the pet types are found by their skeleton files.

use sims_content::{Catalog, Index, Install};
use sims_people::appearance::load_skeleton;
use sims_people::math::Transform;
use sims_people::rig::Pose;
use sims_people::{Assets, Body, Cast, Outfit, attach};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the pet test");
            None
        }
    }
}

/// Every person type whose skeleton is in an underscored file (the
/// pets') loads its skeleton and binds the palm (point 0), the head top
/// (points 1 and 2) and the pelvis (point 3): their skeletons have the
/// bones the body strings name, so what a pet carries is drawn at its
/// paw, as the original's binding (`0x5924a4`) puts it. Each pet
/// skeleton has types whose whole body loads (a type may name a suit the
/// install lacks).
#[test]
fn pets_load_their_skeletons_and_bind_every_attach_point() {
    let Some(root) = install() else { return };
    let found = Install::detect(&root).unwrap();
    let index = Index::build(&found).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let cast = Cast::open(&root, &root.join("UserData")).unwrap();
    // Each pet skeleton, and how many of its types load a whole body.
    let mut bodies = std::collections::BTreeMap::<String, usize>::new();
    for object in catalog.objects.iter().filter(|o| o.kind == 2) {
        let Ok(rows) = cast.body_strings(object.guid) else {
            continue;
        };
        let Some(name) = rows.first() else {
            continue;
        };
        if assets.contains(&format!("{name}-skeleton.cmx.bcf"))
            || !assets.contains(&format!("{name}_skeleton.cmx.bcf"))
        {
            continue;
        }
        let skeleton = load_skeleton(&assets, name)
            .unwrap_or_else(|e| panic!("type {:#010x}: {e}", object.guid));
        let bones = Pose::rest(&skeleton).world(&skeleton, Transform::IDENTITY);
        let points = attach::attach(&skeleton, &bones, &rows);
        assert!(
            points.palm.is_some() && points.head_top.is_some() && points.pelvis.is_some(),
            "type {:#010x}: {points:?}",
            object.guid
        );
        let loaded = Body::load(&assets, name, &Outfit::from_body_strings(&rows))
            .is_ok_and(|body| !body.parts.is_empty());
        *bodies.entry(name.clone()).or_default() += usize::from(loaded);
    }
    // The pets come with the pack in bit 32 of `Expansions::ALL`; an
    // install with it must have both.
    if !found.expansions.contains(32) {
        assert!(bodies.is_empty(), "{bodies:?}");
        eprintln!("this install has no pets; skipping the pet test");
        return;
    }
    // Both kinds of pet (one shared archive brings them), each with
    // bodies that load.
    assert_eq!(bodies.len(), 2, "{bodies:?}");
    assert!(bodies.values().all(|&n| n > 0), "{bodies:?}");
}
