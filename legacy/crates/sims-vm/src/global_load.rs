//! Global instance reconciliation, native 0x442b9f. Factory and deletion services
//! are supplied by the lot host; they must finish before the next resource.
use crate::{Definition, ObjectId};

#[derive(Debug, Clone)]
pub struct Resource {
    pub definition: Definition,
    pub kind: i16,
    pub global: i16,
}
#[derive(Debug, Clone)]
pub struct Instance {
    pub id: ObjectId,
    pub definition: Definition,
    pub global: i16,
}
#[derive(Debug, Clone)]
pub enum Service<'a> {
    Create(&'a Definition),
    Delete(ObjectId),
}

/// `catalog` is resource registration order; `instances` is object registry
/// order. Native snapshots global instances once and never adds newly created
/// objects to that snapshot. Resource identity is IFF + OBJD, not merely GUID.
/// The returned optional value is the AnimTester registry slot (including zero
/// if its factory fails). Missing AnimTester leaves that slot unchanged.
pub fn reconcile(
    catalog: &[Resource],
    instances: &[Instance],
    mut service: impl FnMut(Service<'_>) -> Result<ObjectId, String>,
) -> Result<Option<ObjectId>, String> {
    if instances.iter().any(|i| i.id <= 0) {
        return Err("global reconciliation requires registered positive object IDs".into());
    }
    let mut existing: Vec<_> = instances.iter().filter(|i| i.global != 0).collect();
    let mut animation_tester = None;
    for r in catalog {
        if !matches!(r.kind, 7 | 12) || r.global == 0 {
            continue;
        }
        let mut selected = 0;
        let mut i = 0;
        while i < existing.len() {
            let object = existing[i];
            if object.definition.source == r.definition.source
                && object.definition.object == r.definition.object
            {
                if selected == 0 {
                    selected = object.id;
                } else {
                    service(Service::Delete(object.id))?;
                    existing.remove(i);
                    // Native restarts this scan after erasing a duplicate.
                    selected = 0;
                    i = 0;
                    continue;
                }
            }
            i += 1;
        }
        if selected == 0 {
            selected = service(Service::Create(&r.definition))?;
        }
        if r.definition.guid == 0x3eec206c {
            animation_tester = Some(selected);
        }
    }
    Ok(animation_tester)
}
