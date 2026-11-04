use anyhow::Result;
use sims_content::{Index, skeleton_resources::Cache};
use sims_sim::pose::skeleton::Instance;

#[test]
#[ignore = "requires preserved private Wine Newbie resources"]
fn resource_and_saved_person_rng_checkpoints_match_startup_capture() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/wine-parity-import-1/assets");
    let index = Index::scan(&root)?;
    let mut cache = Cache::default();
    let mut crt = 120733505;
    let adult = cache.load(&index, "adult", &mut crt)?;
    // Original startup-trace-6: resource read, Bob clone/defaults, Betty
    // clone/defaults, then the two built-in root indicator rigs. Each number
    // is an observed boundary, not a seed supplied to the implementation.
    assert_eq!(crt, 1214367985);
    for (clone_end, traits_end) in [(2028189345, 3713050820), (2181756596, 4147029731)] {
        let before = crt;
        assert_eq!(cache.load(&index, "adult", &mut crt)?, adult);
        assert_eq!(crt, before, "cached resource must not construct new bones");
        Instance::clone_prototype(&adult, &mut crt).map_err(anyhow::Error::msg)?;
        assert_eq!(crt, clone_end);
        let mut person = (0..256).map(|n| (n, 0)).collect();
        sims_vm::person_factory::default_traits(&mut person, &mut crt)
            .map_err(anyhow::Error::msg)?;
        assert_eq!(crt, traits_end);
    }
    Instance::root(&mut crt);
    Instance::root(&mut crt);
    assert_eq!(crt, 771473443);
    let mut restored: Cache = serde_json::from_slice(&serde_json::to_vec(&cache)?)?;
    restored.validate(&index)?;
    let before = crt;
    restored.load(&index, "adult", &mut crt)?;
    assert_eq!(crt, before);
    assert_eq!(restored, cache);
    // A different resource is constructed once, independently of the adult
    // cache. This checks cache ownership; native numerical clone coverage lives
    // in the skeleton constructor fixture, not in a made-up warm gameplay run.
    let child = restored.load(&index, "child", &mut crt)?;
    assert_ne!(crt, before);
    let after = crt;
    assert_eq!(restored.load(&index, "child", &mut crt)?, child);
    assert_eq!(crt, after);
    Ok(())
}
