//! Private original clips/rig for native animation event-and-pose comparisons.
use anyhow::{Context, Result};
use serde_json::json;
use sims_content::{Index, saves::House};
use sims_sim::pose::{
    LocalBone, events,
    layer::{Layer, Start},
    sampling::BoneInput,
};
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let house = House::load(&index, "UserData/Houses/House07.iff")?;
    let resident = &house.household.as_ref().context("household")?.residents[0];
    let person = house.objects.instances[&resident.object]
        .person
        .as_ref()
        .context("person")?;
    let avatar = sims_content::saved_avatar::load(&index, &resident.body_strings[0], person)?;
    let mut program = sims_vm::Program::default();
    let mut rows = vec![];
    for name in [
        "a2o-sit-front",
        "a2o-stand-front",
        "a2o-walking-loop",
        "a2o-walking-start",
    ] {
        sims_content::assets::load_named_animations(
            &index,
            &mut program,
            &[name.to_owned()].into(),
        )?;
        let asset = program.animations.get(name).context("clip")?;
        let streams = asset
            .clip
            .motions
            .iter()
            .map(events::stream)
            .collect::<Result<Vec<_>, _>>()
            .map_err(anyhow::Error::msg)?;
        for speed in [-1., 1.] {
            let mut layer = Layer::bind(
                &avatar.skeleton,
                &asset.clip,
                Start {
                    priority: 1,
                    speed,
                    weight: 1.,
                    now: 123456,
                },
            )
            .map_err(anyhow::Error::msg)?;
            layer.clock.mode = i32::from(name == "a2o-walking-loop");
            layer.clock.phase = if speed < 0. { 1. } else { 0. };
            let bones = avatar
                .skeleton
                .bones
                .iter()
                .map(|b| BoneInput {
                    local: LocalBone {
                        translation: b.translation,
                        rotation: b.rotation,
                    },
                    priority: -10000,
                    blend_marker: b.flags[2],
                })
                .collect::<Vec<_>>();
            let hierarchy = avatar
                .skeleton
                .bones
                .iter()
                .map(|b| json!({"name":b.name,"parent":b.parent}))
                .collect::<Vec<_>>();
            rows.push(json!({"layer":layer,"bones":bones,"asset":asset,"streams":streams,"hierarchy":hierarchy,"profile":resident.body_strings}));
        }
    }
    std::fs::write(
        "cache/original-layer-updates-input.json",
        serde_json::to_vec(&rows)?,
    )?;
    println!("Exported {} original clip/direction inputs", rows.len());
    Ok(())
}
