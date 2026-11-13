use super::Progress;
use crate::portrait::{
    cache::{self, Images, Settings},
    job::{Job, Progress as JobProgress},
    retrieval::refresh_service,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::object_catalog::Catalog;
use sims_runtime::original_session::Session;
use sims_sim::Step;
use sims_vm::{World, dialog::Image, image_cache::regeneration::Regeneration};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Target {
    selected: i32,
    selected_guid: u32,
    index: u32,
    guid: u32,
    neighbor: i16,
}

fn target(world: &World, selected: i32) -> Result<Option<Target>> {
    if selected == -1 {
        return Ok(None);
    }
    let entry = world
        .image_cache
        .entries
        .get(usize::try_from(selected)?)
        .context("relationship icon selected cache absent")?;
    let people = world
        .person
        .as_ref()
        .context("relationship icon neighborhood absent")?;
    let selected_people: Vec<_> = people
        .neighbors
        .values()
        .filter(|person| person.guid != 0 && person.guid == entry.guid)
        .collect();
    ensure!(
        selected_people.len() == 1,
        "relationship icon selected identity not unique"
    );
    for (&neighbor, row) in &selected_people[0].relationships {
        if row.is_empty() {
            continue;
        }
        let Some(person) = people.neighbors.get(&neighbor) else {
            continue;
        };
        let indexes: Vec<_> = world
            .image_cache
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.guid != 0 && entry.guid == person.guid)
            .map(|(index, _)| index)
            .collect();
        ensure!(
            indexes.len() <= 1,
            "relationship icon target cache identity ambiguous"
        );
        if let Some(&index) = indexes.first() {
            return Ok(Some(Target {
                selected,
                selected_guid: entry.guid,
                index: u32::try_from(index)?,
                guid: person.guid,
                neighbor,
            }));
        }
    }
    Ok(None)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Relationship {
    target: Target,
    tick: u64,
    job: Option<Job>,
    prepared: bool,
    background: Option<u32>,
    thumbnail: Option<u32>,
    color: u32,
}

impl Relationship {
    pub(super) fn available(session: &Session) -> Result<bool> {
        Ok(target(
            &session.simulation.vm.world,
            session.selected_portrait_index()?,
        )?
        .is_some())
    }

    pub(super) fn new(
        session: &Session,
        background: Option<u32>,
        thumbnail: Option<u32>,
        color: u32,
    ) -> Result<Option<Self>> {
        let Some(target) = target(
            &session.simulation.vm.world,
            session.selected_portrait_index()?,
        )?
        else {
            return Ok(None);
        };
        let entry = &session.simulation.vm.world.image_cache.entries[target.index as usize];
        let prepared = entry.images[2].is_some() && !entry.dirty[2];
        Ok(Some(Self {
            tick: session.simulation.vm.tick,
            job: (!prepared).then(|| {
                Job::new(Regeneration::new(
                    target.index,
                    8,
                    0,
                    background,
                    thumbnail,
                    color,
                ))
            }),
            target,
            prepared,
            background,
            thumbnail,
            color,
        }))
    }

    pub(super) fn validate(&self, session: &Session, images: &Images) -> Result<()> {
        ensure!(
            session.simulation.vm.tick == self.tick
                && target(
                    &session.simulation.vm.world,
                    session.selected_portrait_index()?
                )?
                .as_ref()
                    == Some(&self.target),
            "relationship icon selected/related owner changed"
        );
        ensure!(
            self.prepared == self.job.is_none(),
            "invalid relationship icon preparation state"
        );
        if let Some(job) = &self.job {
            ensure!(
                job.binding()
                    == (
                        self.target.index,
                        8,
                        0,
                        self.background,
                        self.thumbnail,
                        self.color
                    ),
                "relationship icon regeneration binding changed"
            );
            job.validate(session, images)?;
        }
        Ok(())
    }

    pub(super) fn advance(
        &mut self,
        host: &mut cache::Session<'_>,
        catalog: &mut Catalog<'_>,
        images: &mut Images,
        settings: &Settings,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Progress> {
        self.validate(host.session, images)?;
        ensure!(
            self.color == u32::from(settings.key),
            "relationship icon color binding changed"
        );
        while let Some(job) = &mut self.job {
            match job.advance(host, images, settings, remaining, step)? {
                JobProgress::Temporary(service) => return Ok(Progress::Temporary(service)),
                JobProgress::Yielded => return Ok(Progress::Yielded),
                JobProgress::Service(request) => {
                    let response = refresh_service(host.session, catalog, images, request)?;
                    job.respond(host.session, images, response)?;
                }
                JobProgress::Complete(_) => {
                    self.job = None;
                    self.prepared = true;
                }
            }
        }
        Ok(Progress::Complete)
    }

    pub(super) fn finish(
        &self,
        session: &mut Session,
        images: &mut Images,
    ) -> Result<Option<Image>> {
        self.validate(session, images)?;
        ensure!(self.prepared, "relationship icon generation incomplete");
        let Some(handle) =
            session.simulation.vm.world.image_cache.entries[self.target.index as usize].images[2]
        else {
            return Ok(None);
        };
        ensure!(
            images.get(handle)?.rect() == [0, 0, 200, 40],
            "relationship icon strip has invalid dimensions"
        );
        let mut owner = None;
        session
            .simulation
            .vm
            .world
            .image_cache
            .assign_image(&mut owner, Some(handle), &mut |event, state| {
                images.event(event, state)
            })
            .map_err(anyhow::Error::msg)?;
        Ok(owner.map(|handle| Image {
            handle,
            rect: [80, 0, 120, 40],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::{
        image_cache::Entry,
        person::{Neighbor, WorldState},
    };

    #[test]
    fn relationship_icon_binds_selected_and_related_cache_identities() -> Result<()> {
        let mut world = World::default();
        world.image_cache.entries = vec![Entry::new(100), Entry::new(200)];
        let mut selected = Neighbor {
            guid: 100,
            ..Neighbor::default()
        };
        selected.relationships.insert(47, vec![55]);
        world.person = Some(WorldState {
            neighbors: [
                (48, selected),
                (
                    47,
                    Neighbor {
                        guid: 200,
                        ..Neighbor::default()
                    },
                ),
            ]
            .into(),
            ..WorldState::default()
        });
        let expected = target(&world, 0)?.context("related person absent")?;
        ensure!(expected.index == 1 && expected.guid == 200 && expected.neighbor == 47);
        ensure!(target(&world, -1)?.is_none());
        ensure!(target(&world, 3).is_err());
        world.image_cache.entries.push(Entry::new(200));
        ensure!(target(&world, 0).is_err());
        world.image_cache.entries.pop();
        world
            .person
            .as_mut()
            .unwrap()
            .neighbors
            .get_mut(&48)
            .unwrap()
            .relationships
            .clear();
        ensure!(target(&world, 0)?.is_none());
        world.person.as_mut().unwrap().neighbors.insert(
            49,
            Neighbor {
                guid: 100,
                ..Neighbor::default()
            },
        );
        ensure!(target(&world, 0).is_err());
        Ok(())
    }
}
