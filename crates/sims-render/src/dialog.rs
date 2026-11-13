//! Synchronous resource-portrait dialog construction. This owner sits between
//! the suspended VM opcode and the pixel/resource host; ordinary lot work must
//! remain paused until the native constructor returns and publishes its record.
use crate::portrait::{
    cache::{self, Images, Settings},
    retrieval::{Progress as Retrieved, Retrieval},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::object_catalog::Catalog;
use sims_runtime::original_session::Session;
use sims_sim::{Step, portrait_person};
use sims_vm::{
    ThreadId,
    dialog::{Construction, Description, Image},
};
pub mod client;
mod relationship;
mod guid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    thread: ThreadId,
    request: Construction,
    description: Description,
    retrieval: Option<Retrieval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    object_preview: Option<sims_vm::Definition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relationship: Option<relationship::Relationship>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guid: Option<guid::Guid>,
    complete: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Temporary(portrait_person::Service),
    Yielded,
    Complete,
}

impl Pending {
    pub fn begin(
        session: &Session,
        catalog: &mut Catalog<'_>,
        thread: ThreadId,
        background: Option<u32>,
        thumbnail: Option<u32>,
        color: u32,
    ) -> Result<Self> {
        let vm = &session.simulation.vm;
        let request = vm
            .dialog_construction(thread)
            .map_err(anyhow::Error::msg)?
            .clone();
        let description = request
            .describe(
                &session.program,
                &vm.world,
                &mut sims_content::dialog::neighbor::Neighbors {
                    world: vm
                        .world
                        .person
                        .as_ref()
                        .context("dialog neighborhood absent")?,
                    catalog,
                    language: u8::try_from(
                        session
                            .program
                            .text_language
                            .context("dialog language absent")?,
                    )?,
                },
            )
            .map_err(anyhow::Error::msg)?;
        let relationship = if description.relationship {
            relationship::Relationship::new(session, background, thumbnail, color)?
        } else {
            None
        };
        let guid = match (description.record.guid_icon, description.resource) {
            (Some(guid), Some(resource)) => {
                Some(guid::Guid::new(guid, resource, catalog, color)?)
            }
            _ => None,
        };
        let mut object_preview = None;
        let retrieval = if description.record.guid_icon.is_some() {
            None
        } else if let Some(resource) = description.resource {
            catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            let entry = &catalog.definitions.entries[resource as usize - 1];
            ensure!(
                entry.data.len() == 216,
                "dialog definition is not normalized"
            );
            let kind = u16::from_le_bytes(entry.data[18..20].try_into().unwrap());
            let guid = u32::from_le_bytes(entry.data[28..32].try_into().unwrap());
            if kind != 2 {
                object_preview = Some(
                    catalog
                        .portrait_definition(resource)
                        .map_err(anyhow::Error::msg)?,
                );
                None
            } else {
                vm.world
                    .image_cache
                    .entries
                    .iter()
                    .position(|e| e.guid == guid)
                    .map(|index| {
                        Retrieval::new(vm.tick, index as u32, None, background, thumbnail, color)
                    })
            }
        } else {
            None
        };
        Ok(Self {
            thread,
            request,
            description,
            retrieval,
            object_preview,
            relationship,
            guid,
            complete: false,
        })
    }
    pub fn record(&self) -> &sims_vm::dialog::Record {
        &self.description.record
    }
    pub fn validate(&self, session: &Session, images: &Images) -> Result<()> {
        ensure!(!self.complete, "dialog constructor already returned");
        // The original thread remains suspended even while a temporary person's
        // separate thread runs its synchronous construction/removal callbacks.
        ensure!(
            session
                .simulation
                .vm
                .dialog_construction(self.thread)
                .map_err(anyhow::Error::msg)?
                == &self.request,
            "dialog constructor binding changed"
        );
        ensure!(
            self.request.tick() == session.simulation.vm.tick,
            "dialog constructor advanced the clock"
        );
        ensure!(
            self.description.record.id == self.request.id(),
            "dialog record identity changed"
        );
        images.validate(&session.simulation.vm.world.image_cache)?;
        let relationship_icon = self.request
            .relationship_icon(&session.program, &session.simulation.vm.world)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.description.relationship == relationship_icon,
            "dialog relationship icon binding changed"
        );
        ensure!(
            self.description.record.guid_icon
                == self.request.guid_icon(&session.program, &session.simulation.vm.world)
                    .map_err(anyhow::Error::msg)?,
            "dialog GUID request binding changed"
        );
        ensure!(
            self.guid.is_some()
                == (self.description.record.guid_icon.is_some() && self.description.resource.is_some()),
            "dialog GUID continuation absent or unowned"
        );
        if let Some(guid) = &self.guid {
            ensure!(
                self.retrieval.is_none() && self.object_preview.is_none() && self.relationship.is_none(),
                "overlapping dialog GUID image owners"
            );
            guid.validate(self.description.record.guid_icon, self.description.resource)?;
        }
        ensure!(
            self.relationship.is_some()
                == (self.description.relationship && relationship::Relationship::available(session)?),
            "dialog relationship continuation absent or unowned"
        );
        if let Some(relationship) = &self.relationship {
            ensure!(
                self.retrieval.is_none() && self.object_preview.is_none(),
                "overlapping dialog image owners"
            );
            relationship.validate(session, images)?;
        }
        ensure!(
            self.object_preview.is_none()
                || (self.retrieval.is_none()
                    && self.description.resource.is_some()
                    && self.description.record.portrait.is_none()),
            "invalid pending object preview owner"
        );
        if let Some(retrieval) = &self.retrieval {
            retrieval.validate(session, images)?;
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        host: &mut cache::Session<'_>,
        catalog: &mut Catalog<'_>,
        images: &mut Images,
        settings: &Settings,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Progress> {
        images.collect(&host.session.simulation.vm.world.image_cache)?;
        self.validate(host.session, images)?;
        if let Some(guid) = &self.guid {
            if *remaining == 0 {
                return Ok(Progress::Yielded);
            }
            let raster = guid.render(catalog, host.session.index(), settings)?;
            let handle = images.insert(&mut host.session.simulation.vm.world.image_cache, raster)?;
            self.description.record.portrait = Some(Image {
                handle,
                rect: [0, 0, 45, 45],
            });
            *remaining -= 1;
        }
        if let Some(relationship) = &mut self.relationship {
            match relationship.advance(host, catalog, images, settings, remaining, step)? {
                Progress::Temporary(service) => return Ok(Progress::Temporary(service)),
                Progress::Yielded => return Ok(Progress::Yielded),
                Progress::Complete => {}
            }
            self.description.record.portrait = relationship.finish(host.session, images)?;
            self.relationship = None;
        }
        if let Some(definition) = &self.object_preview {
            if *remaining == 0 {
                return Ok(Progress::Yielded);
            }
            let resource = self
                .description
                .resource
                .context("object preview resource absent")?;
            ensure!(
                catalog
                    .portrait_definition(resource)
                    .map_err(anyhow::Error::msg)?
                    == *definition,
                "object preview resource identity changed"
            );
            // Render before allocating a shared image handle. An unsupported
            // codec/projection leaves the suspended constructor and VM intact.
            let (raster, _) = crate::object_preview::software::dialog(
                catalog,
                resource,
                settings.format,
                settings.key,
            )?;
            let handle =
                images.insert(&mut host.session.simulation.vm.world.image_cache, raster)?;
            self.description.record.portrait = Some(Image {
                handle,
                rect: [0, 0, 120, 120],
            });
        }
        if let Some(retrieval) = &mut self.retrieval {
            match retrieval.advance(host, catalog, images, settings, remaining, step)? {
                Retrieved::Temporary(service) => return Ok(Progress::Temporary(service)),
                Retrieved::Yielded => return Ok(Progress::Yielded),
                Retrieved::Complete(_) => {}
            }
            let (success, mut output) = retrieval.clone().finish()?;
            if success {
                self.description.record.portrait = output.map(|handle| Image {
                    handle,
                    rect: [0, 0, 45, 45],
                });
            } else {
                images.release(
                    &mut host.session.simulation.vm.world.image_cache,
                    &mut output,
                )?;
            }
            self.retrieval = None;
        }
        let published = host
            .session
            .simulation
            .vm
            .finish_dialog_construction(
                &host.session.program,
                self.thread,
                self.description.record.clone(),
            )
            .map_err(anyhow::Error::msg);
        if let Err(error) = published {
            // A rejected record must not leak the local image wrapper. The
            // constructor remains pending, with no published image ownership.
            if let Some(image) = self.description.record.portrait.take() {
                images.release(
                    &mut host.session.simulation.vm.world.image_cache,
                    &mut Some(image.handle),
                )?;
            }
            return Err(error);
        }
        // Ownership has moved into the published record. The completed driver
        // retains text only and cannot transfer the same image a second time.
        self.description.record.portrait = None;
        self.object_preview = None;
        self.guid = None;
        self.complete = true;
        Ok(Progress::Complete)
    }
}
