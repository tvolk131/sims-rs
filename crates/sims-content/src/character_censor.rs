//! Asset/geometry owner for 58faf6's direct masked appearance binding. These
//! instances are separate from clothing/accessory records and share their ID
//! allocator and native geometry lifecycle. One complete service commits at a
//! time; decode errors leave the pending request and all owners unchanged.
use crate::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    character_model,
    saved_avatar::BoundAvatar,
};
use anyhow::{Result, ensure};
use sims_sim::{
    character::Resident,
    pose::{
        appearance_reload::Owner,
        refresh::{Request, Response},
    },
};

pub fn validate(
    resident: &Resident,
    owner: &Owner,
    avatar: &BoundAvatar,
    geometry: &CharacterGeometry,
) -> Result<()> {
    let censor = &resident.character.censor;
    ensure!(
        owner.attachment == censor.binding && owner.attachment_metadata == censor.mask as u32,
        "censor animator and appearance owner disagree"
    );
    validate_assets(owner, avatar, geometry)
}

/// During an appearance reload its continuation owns the current attachment;
/// the resident's cached censor fields are published when that job completes.
pub fn validate_assets(
    owner: &Owner,
    avatar: &BoundAvatar,
    geometry: &CharacterGeometry,
) -> Result<()> {
    ensure!(
        avatar.auxiliary().len() == usize::from(owner.attachment.is_some()),
        "censor auxiliary binding ownership mismatch"
    );
    if let Some(id) = owner.attachment {
        let binding = avatar
            .auxiliary()
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("censor assets absent"))?;
        ensure!(
            binding.mask == owner.attachment_metadata && geometry.appearances.contains_key(&id),
            "censor geometry or mask mismatch"
        );
    }
    Ok(())
}

pub fn service(
    index: &Index,
    resident: &mut Resident,
    model: &mut character_model::State,
    geometry: &mut CharacterGeometry,
    avatar: &mut BoundAvatar,
    owner: &mut Owner,
    pool: &mut MeshPool,
) -> Result<bool> {
    let Some(request) = resident.character.pending_request().cloned() else {
        return Ok(false);
    };
    if !matches!(
        request,
        Request::BindCensor { .. } | Request::UnbindCensor { .. }
    ) {
        return Ok(false);
    }
    validate(resident, owner, avatar, geometry)?;
    let (
        mut next_resident,
        mut next_model,
        mut next_geometry,
        mut next_avatar,
        mut next_owner,
        mut next_pool,
    ) = (
        resident.clone(),
        model.clone(),
        geometry.clone(),
        avatar.clone(),
        owner.clone(),
        pool.clone(),
    );
    let response = match request {
        Request::BindCensor { mask } => {
            ensure!(
                mask != 0 && next_resident.character.censor.binding.is_none(),
                "censor bind requires a nonzero mask and no previous binding"
            );
            let name = next_model.needs_appearance.clone();
            let id = next_model.with_bound(&mut next_resident, |bound| {
                next_geometry.bind_auxiliary(
                    index,
                    &mut next_avatar,
                    &name,
                    mask as u32,
                    bound,
                    &mut next_pool,
                )
            })?;
            Response::CensorBinding(Some(id))
        }
        Request::UnbindCensor { binding } => {
            ensure!(
                next_resident.character.censor.binding == Some(binding),
                "censor unbind identity changed"
            );
            next_model.with_bound(&mut next_resident, |bound| {
                next_geometry.detach_auxiliary(
                    index,
                    &mut next_avatar,
                    binding,
                    bound,
                    &mut next_pool,
                )
            })?;
            Response::Done
        }
        _ => unreachable!(),
    };
    next_resident
        .character
        .acknowledge(response)
        .map_err(anyhow::Error::msg)?;
    next_owner.attachment = next_resident.character.censor.binding;
    next_owner.attachment_metadata = next_resident.character.censor.mask as u32;
    validate(&next_resident, &next_owner, &next_avatar, &next_geometry)?;
    (*resident, *model, *geometry, *avatar, *owner, *pool) = (
        next_resident,
        next_model,
        next_geometry,
        next_avatar,
        next_owner,
        next_pool,
    );
    Ok(true)
}
