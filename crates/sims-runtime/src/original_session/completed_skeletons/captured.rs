//! Join current captured rows without the historical projector's tick pin.
//! Does not start a pose, initialize a skeleton, or qualify synchronous absence.
use super::*;
use sims_fixtures::readset::Readset;

/// Check every source receipt against the actual immutable sparse observation.
/// Missing bytes remain errors; this cannot extend an observed readset.
pub(crate) fn reads(input: &Value, memory: &Readset<'_>) -> Result<()> {
    let receipts = input["reads"]
        .as_array()
        .context("captured character reads absent")?;
    ensure!(
        !receipts.is_empty(),
        "captured character read receipts empty"
    );
    for r in receipts {
        let p = u32::try_from(
            r["address"]
                .as_u64()
                .context("character read address absent")?,
        )?;
        let n = usize::try_from(
            r["bytes"]
                .as_u64()
                .context("character read extent absent")?,
        )?;
        ensure!(n <= 16 << 20, "character read exceeds bound");
        let bytes = memory.read(p, n)?;
        ensure!(
            r["sha256"] == sims_fixtures::digest(&bytes),
            "captured character read differs"
        );
    }
    Ok(())
}

fn index<'a>(input: &'a Value, name: &str, key: &str) -> Result<BTreeMap<u32, &'a Value>> {
    let mut out = BTreeMap::new();
    for row in input[name]
        .as_array()
        .context("captured skeleton rows absent")?
    {
        let p = u32_(field(row, key)?)?;
        ensure!(
            p != 0 && out.insert(p, row).is_none(),
            "aliased captured skeleton row"
        );
    }
    Ok(out)
}

fn storage(row: &Value, memory: &Readset<'_>) -> Result<()> {
    let p = u32_(field(row, "pointer")?)?;
    let raw = super::super::completed_animations::bytes(
        field(row, "raw")?.as_str().context("skeleton raw absent")?,
    )?;
    ensure!(
        raw.len() <= 1 << 20 && memory.read(p, raw.len())? == raw,
        "captured skeleton storage differs from observed memory"
    );
    Ok(())
}

fn vector(row: &Value, memory: &Readset<'_>) -> Result<()> {
    let p = u32_(field(row, "begin")?)?;
    let raw = super::super::completed_animations::bytes(
        field(row, "used_raw")?
            .as_str()
            .context("skeleton vector raw absent")?,
    )?;
    ensure!(
        raw.len() <= 1 << 20 && memory.read(p, raw.len())? == raw,
        "captured skeleton vector differs from observed memory"
    );
    Ok(())
}

pub fn import(
    state: &Value,
    animators: BTreeMap<ObjectId, Animator>,
    memory: &Readset<'_>,
    fresh_epoch: &str,
) -> Result<BTreeMap<ObjectId, Imported>> {
    let sk = field(state, "skeletons")?;
    let chars = field(state, "characters")?;
    reads(sk, memory)?;
    reads(chars, memory)?;
    let characters = index(chars, "characters", "object")?;
    let skins = index(sk, "skins", "pointer")?;
    let bones = index(sk, "bones", "pointer")?;
    let anchors = index(sk, "anchors", "pointer")?;
    for row in skins.values() {
        storage(row, memory)?;
        vector(field(row, "bone_vector")?, memory)?;
        vector(field(row, "lookup_vector")?, memory)?;
    }
    for row in bones.values() {
        storage(row, memory)?;
        vector(field(row, "name")?, memory)?;
        vector(field(row, "parent_name")?, memory)?;
    }
    for row in anchors.values() {
        storage(row, memory)?;
    }
    let mut used_skins = BTreeSet::new();
    let mut used_bones = BTreeSet::new();
    let mut used_anchors = BTreeSet::new();
    let mut used_characters = BTreeSet::new();
    let mut rows = Vec::new();
    for c in sk["characters"]
        .as_array()
        .context("captured skeleton characters absent")?
    {
        let id = u32_(field(c, "object")?)?;
        let skin_pointer = u32_(field(c, "skin")?)?;
        ensure!(
            used_skins.insert(skin_pointer) && used_characters.insert(id),
            "aliased mutable captured skeleton"
        );
        let skin = *skins.get(&skin_pointer).context("captured skin absent")?;
        let mut ordered = Vec::new();
        for p in skin["ordered_bones"]
            .as_array()
            .context("captured bone order absent")?
        {
            let p = u32_(p)?;
            ensure!(used_bones.insert(p), "aliased captured mutable bone");
            ordered.push((*bones.get(&p).context("captured bone absent")?).clone());
        }
        let mut ap = BTreeSet::new();
        let mut selected = Vec::new();
        for name in ["head", "hand", "pelvis", "forward"] {
            let p = u32_(field(field(c, "anchors")?, name)?)?;
            if p != 0 && ap.insert(p) {
                // A hand and head may refer to the same anchor within one rig,
                // but separate characters cannot own the same mutable anchor.
                ensure!(used_anchors.insert(p), "aliased captured mutable anchor");
                selected.push((*anchors.get(&p).context("captured anchor absent")?).clone());
            }
        }
        rows.push(Input {
            character: c.clone(),
            animator: (*characters.get(&id).context("captured animator absent")?).clone(),
            skin: skin.clone(),
            bones: ordered,
            anchors: selected,
            code: sk["code"]
                .as_array()
                .context("captured skeleton code absent")?
                .clone(),
        });
    }
    ensure!(
        used_skins.iter().eq(skins.keys())
            && used_bones.iter().eq(bones.keys())
            && used_anchors.iter().eq(anchors.keys())
            && used_characters.iter().eq(characters.keys()),
        "captured skeleton source coverage differs"
    );
    let mut imported = super::import(&rows, animators, fresh_epoch)?;
    // Animator has no native FCW field. A completed whole-process capture must
    // bind its observed arithmetic environment rather than inherit the isolated
    // probe default used by historical component-only inputs.
    let controls: Vec<_> = state["identity"]["hardware_registers"]
        .as_str()
        .context("current skeleton CPU control absent")?
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("FCW="))
        .collect();
    ensure!(
        controls.len() == 1,
        "current skeleton CPU control missing/ambiguous"
    );
    let control = u16::from_str_radix(controls[0], 16)?;
    for part in imported.values_mut() {
        part.animator
            .bind_fp_control(control)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(imported)
}
