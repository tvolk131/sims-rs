//! Person+aec owns its current score curves. Global STR# resources are inputs
//! to reset, not replacements for this live cache at each autonomy query.
use super::Curve;
use crate::{ObjectId, Program, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCurve {
    pub motive: usize,
    pub points: Vec<[u32; 2]>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub curves: Vec<StoredCurve>,
}
impl Owner {
    pub fn capture(curves: &[Curve]) -> Result<Self, String> {
        if curves.is_empty() || curves.len() > 16 {
            return Err("invalid native personal score curve count".into());
        }
        for c in curves {
            c.validate()?;
        }
        Ok(Self {
            curves: curves
                .iter()
                .map(|c| StoredCurve {
                    motive: c.motive,
                    points: c.points.iter().map(|p| p.map(f32::to_bits)).collect(),
                })
                .collect(),
        })
    }
    pub fn values(&self) -> Result<Vec<Curve>, String> {
        let curves: Vec<_> = self
            .curves
            .iter()
            .map(|c| Curve {
                motive: c.motive,
                points: c.points.iter().map(|p| p.map(f32::from_bits)).collect(),
            })
            .collect();
        Self::capture(&curves)?;
        Ok(curves)
    }
}
pub fn current(world: &World, p: &Program, actor: ObjectId) -> Result<Vec<Curve>, String> {
    let object = world
        .objects
        .get(&actor)
        .ok_or("personal score actor absent")?;
    if let Some(owner) = &object.native_motive_scores {
        return owner.values();
    }
    if object.native_flags.is_some() || world.scheduler_storage.is_some() {
        return Err("native personal score curve owner unavailable".into());
    }
    // Authored VM fixtures have no native object/registry lifetime. Keep their
    // resource-defined test profile explicit; original callers require above.
    let age = *object
        .person
        .as_ref()
        .and_then(|v| v.get(&58))
        .ok_or("personal score age absent")?;
    let tuning = super::Tuning::from_program(p)?;
    Ok(if age == 0 || age > 17 {
        tuning.adult_scores
    } else {
        tuning.child_scores
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_cache_wins_without_reading_program_tuning_and_missing_is_unknown() {
        let curves = vec![Curve {
            motive: 5,
            points: vec![[-0.0, 10.0], [100.0, 20.0]],
        }];
        let owner = Owner::capture(&curves).unwrap();
        let mut world = World::default();
        world.objects.insert(
            1,
            crate::Object {
                native_flags: Some(0),
                native_motive_scores: Some(owner.clone()),
                ..Default::default()
            },
        );
        let program = Program::default();
        assert_eq!(current(&world, &program, 1).unwrap(), curves);
        let restored: Owner = serde_json::from_slice(&serde_json::to_vec(&owner).unwrap()).unwrap();
        assert_eq!(restored, owner);
        assert_eq!(restored.curves[0].points[0][0], (-0.0f32).to_bits());
        world.objects.get_mut(&1).unwrap().native_motive_scores = None;
        assert_eq!(
            current(&world, &program, 1).unwrap_err(),
            "native personal score curve owner unavailable"
        );
    }
}
