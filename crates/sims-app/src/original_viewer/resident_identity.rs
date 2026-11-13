use sims_content::saves::Resident;
use sims_vm::World;

pub(super) fn object(world: &World, resident: &Resident) -> Option<i16> {
    let people = world.person.as_ref()?;
    let family = people.families.get(&people.controlled_family)?;
    if !family.contains(&resident.neighbor)
        || people.neighbors.get(&resident.neighbor)?.guid != resident.guid
    {
        return None;
    }
    let mut matching = world.objects.iter().filter_map(|(identity, object)| {
        object.person.as_ref()?;
        let definition = object.definition.as_ref()?;
        (definition.guid == resident.guid && definition.source == resident.character_source)
            .then_some(*identity)
    });
    let identity = matching.next()?;
    matching.next().is_none().then_some(identity)
}

pub(super) fn live<'resident>(
    world: &World,
    residents: &'resident [Resident],
    identity: i16,
) -> Option<&'resident Resident> {
    let definition = world.objects.get(&identity)?.definition.as_ref()?;
    let resident = residents.iter().find(|resident| {
        resident.guid == definition.guid && resident.character_source == definition.source
    })?;
    (object(world, resident) == Some(identity)).then_some(resident)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::{
        Definition, Object,
        person::{Neighbor, WorldState},
    };

    fn resident() -> Resident {
        Resident {
            guid: 844195932,
            neighbor: 47,
            object: 137,
            name: "Betty".into(),
            character_source: "UserData/Characters/User00001.iff".into(),
            body_strings: Vec::new(),
        }
    }

    fn world() -> World {
        let resident = resident();
        let mut people = WorldState::default();
        people.controlled_family = 1;
        people.families.insert(1, vec![resident.neighbor]);
        people.neighbors.insert(
            resident.neighbor,
            Neighbor {
                guid: resident.guid,
                ..Default::default()
            },
        );
        let mut world = World::default();
        world.person = Some(people);
        world.objects.insert(
            134,
            Object {
                definition: Some(Definition {
                    source: resident.character_source,
                    object: 128,
                    guid: resident.guid,
                    slots: 0,
                }),
                person: Some(Default::default()),
                ..Default::default()
            },
        );
        world
    }

    #[test]
    fn resident_identity_resolves_recreated_person_without_mutating_import_roster() {
        let world = world();
        let residents = [resident()];
        let before = serde_json::to_value(&residents).unwrap();
        assert_eq!(object(&world, &residents[0]), Some(134));
        assert_eq!(live(&world, &residents, 134).unwrap().neighbor, 47);
        assert!(live(&world, &residents, 137).is_none());
        assert_eq!(serde_json::to_value(&residents).unwrap(), before);
    }

    #[test]
    fn resident_identity_rejects_reused_objects_sources_and_nonpeople() {
        let mut world = world();
        let residents = [resident()];
        let mut reused = world.objects[&134].clone();
        reused.definition.as_mut().unwrap().guid = 999;
        world.objects.insert(137, reused);
        assert!(live(&world, &residents, 137).is_none());
        assert_eq!(object(&world, &residents[0]), Some(134));
        world
            .objects
            .get_mut(&134)
            .unwrap()
            .definition
            .as_mut()
            .unwrap()
            .source = "UserData2/Characters/User00001.iff".into();
        assert!(object(&world, &residents[0]).is_none());
        world
            .objects
            .get_mut(&134)
            .unwrap()
            .definition
            .as_mut()
            .unwrap()
            .source = residents[0].character_source.clone();
        world.objects.get_mut(&134).unwrap().person = None;
        assert!(object(&world, &residents[0]).is_none());
    }

    #[test]
    fn resident_identity_requires_actual_neighbor_and_controlled_family_membership() {
        let mut world = world();
        let resident = resident();
        world
            .person
            .as_mut()
            .unwrap()
            .neighbors
            .get_mut(&47)
            .unwrap()
            .guid = 999;
        assert!(object(&world, &resident).is_none());
        world
            .person
            .as_mut()
            .unwrap()
            .neighbors
            .get_mut(&47)
            .unwrap()
            .guid = resident.guid;
        world
            .person
            .as_mut()
            .unwrap()
            .families
            .get_mut(&1)
            .unwrap()
            .clear();
        assert!(object(&world, &resident).is_none());
        world.person.as_mut().unwrap().controlled_family = 0;
        world.person.as_mut().unwrap().families.insert(0, vec![47]);
        assert_eq!(object(&world, &resident), Some(134));
    }

    #[test]
    fn resident_identity_rejects_missing_and_ambiguous_live_instances() {
        let mut world = world();
        let resident = resident();
        world.objects.insert(137, world.objects[&134].clone());
        assert!(object(&world, &resident).is_none());
        assert!(live(&world, &[resident.clone()], 134).is_none());
        world.objects.clear();
        assert!(object(&world, &resident).is_none());
    }
}
