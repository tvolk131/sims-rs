//!Live4e989b adapter and CPState+1ec binding, using the actual image cache,
//!resource identities, registry order and selection lifecycle.
use super::Session;
use anyhow::{Context, Result, ensure};
use sims_vm::{ObjectId, interface::person_selection as native};

///4e986d's live getter over the same simulation used by the scheduler.
pub(super) fn selected_index(simulation:&sims_sim::Simulation)->Result<i32> {
    if simulation.vm.world.person.is_none() {return Ok(-1);}
    let Some(id)=simulation.native_selected_person().map_err(anyhow::Error::msg)? else{return Ok(-1);};
    let world=&simulation.vm.world;
    let guid=world.objects.get(&id).and_then(|o|o.definition.as_ref()).context("selected person definition absent")?.guid;
    Ok(native::cache_index(&world.image_cache,guid))
}

struct Live<'a>(&'a mut Session, Option<u32>);
impl native::View for Live<'_> {
    fn cache_guid(&self, index: u32) -> Result<u32, String> {
        Ok(native::cache_guid(
            &self.0.simulation.vm.world.image_cache,
            index,
        ))
    }
    fn registry_present(&self) -> bool {
        self.0.simulation.vm.world.person.is_some()
    }
    fn first_instance(&mut self, guid: u32) -> Result<Option<ObjectId>, String> {
        let s = &mut self.0;
        //Program's TATT storage catalog excludes character resources. Resolve
        //the registered OBJD in the loaded modules, as the VM's shared resource
        //instance service does. Unloaded/ambiguous resources remain errors.
        let mut matches = s.program.modules.values().flat_map(|m| {
            m.object_definition_words
                .iter()
                .filter_map(move |(&id, w)| {
                    (w.len() > 15
                        && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid)
                        .then_some((&m.source, id))
                })
        });
        let resource = matches.next().ok_or("selection GUID resource absent")?;
        if matches.next().is_some() {
            return Err("selection GUID resource ambiguous".into());
        }
        let world = &mut s.simulation.vm.world;
        let registry = world.person.as_mut().ok_or("person registry absent")?;
        //445857 builds the missing vector by numeric object-registry slot,
        //matching the actual resource pointer, not merely the GUID.
        let instances = registry.resource_instances.entry(guid).or_insert_with(|| {
            world
                .objects
                .iter()
                .filter_map(|(&id, o)| {
                    o.definition
                        .as_ref()
                        .filter(|d| &d.source == resource.0 && d.object == resource.1)
                        .map(|_| id)
                })
                .collect()
        });
        Ok(instances.first().copied())
    }
    fn zoning(&self) -> Result<i32, String> {
        //42b9e3: unknown neighborhood lot is zero, as in the shared lot helpers.
        let lot = self.lot_number()?;
        if lot == 0 {
            return Ok(1);
        }
        Ok(self
            .0
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .ok_or("person registry absent")?
            .lot_zoning
            .get(&lot)
            .copied()
            .unwrap_or(0))
    }
    fn lot_number(&self) -> Result<i16, String> {
        self.0
            .simulation
            .vm
            .world
            .globals
            .get(&10)
            .copied()
            .ok_or("lot number absent".into())
    }
    fn selection_override(&self) -> Result<u32, String> {
        self.1
            .or(self.0.host.presentation.person_selection_override)
            .ok_or("selection override unbound".into())
    }
    fn person_data(&self, id: ObjectId, index: i16) -> Result<i16, String> {
        self.0
            .simulation
            .vm
            .world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.get(&index))
            .copied()
            .ok_or("selection person field absent".into())
    }
    fn controlled_family(&self) -> Result<i32, String> {
        Ok(i32::from(
            self.0
                .simulation
                .vm
                .world
                .person
                .as_ref()
                .ok_or("person registry absent")?
                .controlled_family,
        ))
    }
}
impl Session {
    ///4f1ae9's ordinary residential branch republishes the retained FAMI
    ///balance after428041/F6. The caller dirties the CPState funds panel next.
    ///Travel and family-move branches have additional owners, outside this entry.
    pub fn publish_opening_budget(&mut self) -> Result<()> {
        ensure!(
            !self.load_completion_pending()
                && !self.postload_pending()
                && !self.house_notification_pending()
                && !self.simulation.selection_work_pending(),
            "opening budget overlaps synchronous work"
        );
        let family = self
            .house
            .household
            .as_ref()
            .context("opening household absent")?;
        let world = &mut self.simulation.vm.world;
        let people = world
            .person
            .as_ref()
            .context("opening family registry absent")?;
        let lot = world.globals[&10];
        ensure!(
            people.controlled_family == family.id as i16 && people.lot_zoning.get(&lot) == Some(&1),
            "opening budget requires the retained residential family"
        );
        world
            .publish_family_funds(family.family.budget)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
    /// Ordinary4f1ae9 opening tail (4f23e3..4f2410), after F6 and portrait
    ///cache construction. Keep a selected cached person with a nonzero family;
    ///otherwise select controlled-family member zero through4e97dc/4e989b.
    ///The caller owns the process selection override and drives the existing
    ///selection/notification continuation before allowing simulation ticks.
    pub fn begin_opening_selection(&mut self, selection_override: u32) -> Result<bool> {
        use native::View;
        let selected = self.selected_portrait_index()?;
        if selected != -1 {
            let mut view = Live(self, Some(selection_override));
            let guid = view
                .cache_guid(selected as u32)
                .map_err(anyhow::Error::msg)?;
            let person = view
                .first_instance(guid)
                .map_err(anyhow::Error::msg)?
                .context("selected portrait has no person instance")?;
            if view.person_data(person, 61).map_err(anyhow::Error::msg)? != 0 {
                return Ok(false);
            }
        }
        let people = self
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .context("opening family registry absent")?;
        let first = people
            .families
            .get(&people.controlled_family)
            .and_then(|family| family.first())
            .map(|id| {
                people
                    .neighbors
                    .get(id)
                    .context("opening family member absent")
            })
            .transpose()?;
        let index = first.map_or(-1, |person| {
            native::cache_index(&self.simulation.vm.world.image_cache, person.guid)
        });
        let decision = native::select(&mut Live(self, Some(selection_override)), index as u32)
            .map_err(anyhow::Error::msg)?;
        if let native::Decision::Select(target) = decision {
            self.begin_selection(target)?;
            return Ok(true);
        }
        Ok(false)
    }
    ///4e986d ->4456e4 ->resource GUID ->4e9753. This is a live getter,
    ///distinct from the retained CPState index and the presentation selection.
    pub fn selected_portrait_index(&self) -> Result<i32> {
        selected_index(&self.simulation)
    }
    ///Explicit frontend process binding, followed by4ce81c's dirty8 selected
    ///field refresh. A historical pending4e989b argument cannot be rewritten.
    pub fn bind_person_publication(&mut self, selection_override: u32) -> Result<()> {
        use sims_vm::interface::world_publication::Request;
        ensure!(
            self.host.presentation.person_selection_override.is_none()
                && !self.simulation.selection_work_pending()
                && !self.house_notification_pending()
                && !self.move_tool_pending()
                && !self.tool_selection_pending(),
            "selection publication already bound or busy"
        );
        if self.publication_pending() {
            ensure!(
                matches!(
                    self.host
                        .presentation
                        .publication
                        .as_ref()
                        .unwrap()
                        .request(),
                    Some(Request::SelectTool(_) | Request::ActivateTool { .. })
                ),
                "selection index already captured or publication not at a binding boundary"
            );
        } else {
            ensure!(
                !self.player_input_pending()
                    && !self.camera_command_pending()
                    && !self.clock_pending()
                    && !self.interface_update_pending()
                    && !self.control_panel_pending()
                    && !self.ui_wait_pending()
                    && !self.ui_portrait_pending()
                    && !self.view_refresh_pending()
                    && !self.load_completion_pending()
                    && !self.postload_pending(),
                "selection binding overlaps synchronous work"
            );
        }
        let controls = &self
            .host
            .presentation
            .transport
            .as_ref()
            .context("CPState absent")?
            .controls;
        let index = if controls.buy.dirty & 8 != 0 {
            self.selected_portrait_index()?
        } else {
            controls
                .selected_person_index
                .context("CPState selected index unbound")?
        };
        self.host
            .presentation
            .transport
            .as_mut()
            .unwrap()
            .controls
            .selected_person_index = Some(index);
        self.host.presentation.person_selection_override = Some(selection_override);
        Ok(())
    }
    pub(super) fn person_publication_decision(&mut self, index: u32) -> Result<native::Decision> {
        native::select(&mut Live(self, None), index).map_err(anyhow::Error::msg)
    }
}
