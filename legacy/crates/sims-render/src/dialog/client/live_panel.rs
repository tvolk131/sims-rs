//! Actual replacement Iced widgets with original script-visible identities.
//! This owns Needs selection and the visible motive-detail popup. It does not
//! claim to execute the entire legacy51ce79/51c5a6 Windows panel frame. That
//! frame's resumable cores retain their explicit, unimplemented child services.
use super::*;
use sims_vm::{
    ui_effect::widgets::button,
    ui_owner::wait::{Condition, Observation},
};

pub const MOOD_BUTTON: i32 = 4500;
pub const MOTIVES: [(&str, usize); 8] = [
    ("Hunger", 7),
    ("Energy", 5),
    ("Comfort", 6),
    ("Hygiene", 8),
    ("Bladder", 9),
    ("Fun", 15),
    ("Social", 14),
    ("Room", 13),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Person {
    pub object: i16,
    pub guid: u32,
    pub cache_index: u32,
    pub neighbor: i16,
    pub name: String,
    pub values: [u32; 8],
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Panel {
    tick: u64,
    person: Option<Person>,
    state: i32,
    shown: bool,
    popup: Option<u8>,
    /// Replacement widget invalidations. Iced rebuilds the affected view after
    /// the owning input/update; the native setter's equal-state branch is quiet.
    invalidations: u64,
}
impl Panel {
    pub fn person(&self) -> Option<&Person> {
        self.person.as_ref()
    }
    pub fn selected(&self) -> bool {
        self.state != 0
    }
    pub fn shown(&self) -> bool {
        self.shown
    }
    pub fn popup(&self) -> Option<usize> {
        self.popup.map(usize::from)
    }
    pub fn observation(&self, condition: &Condition) -> Option<Observation> {
        match condition {
            Condition::Control(MOOD_BUTTON) => Some(Observation::Control {
                image: Some(MOOD_BUTTON),
                state: self.state as u32,
            }),
            Condition::Panel(_) | Condition::PanelAbsent => Some(Observation::Panel(
                self.popup.map(|_| "motives".to_string()),
            )),
            _ => None,
        }
    }
    pub(super) fn validate(&self, session: &Session) -> Result<()> {
        ensure!(
            self.tick <= session.simulation.vm.tick,
            "live widgets are from a future tick"
        );
        ensure!(
            matches!(self.state, 0 | 1) && self.shown == (self.state != 0),
            "needs widget publication differs"
        );
        ensure!(
            self.state == i32::from(self.person.is_some()),
            "needs selection has no displayed person"
        );
        ensure!(
            self.popup.is_none_or(|n| n < 8 && self.shown),
            "motive popup lost its widget"
        );
        if let Some(person) = &self.person {
            let entry = session
                .simulation
                .vm
                .world
                .image_cache
                .entries
                .get(person.cache_index as usize)
                .context("displayed person's image-cache entry absent")?;
            ensure!(
                entry.guid == person.guid,
                "displayed person's cache identity changed"
            );
            ensure!(
                session
                    .simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .and_then(|p| p.neighbors.get(&person.neighbor))
                    .is_some_and(|n| n.guid == person.guid),
                "displayed person's neighbor identity changed"
            );
            ensure!(
                person.object > 0 && person.guid != 0 && person.neighbor > 0,
                "invalid displayed person identity"
            );
            ensure!(
                person.values.iter().all(|v| f32::from_bits(*v).is_finite()),
                "non-finite displayed motive"
            );
        }
        Ok(())
    }
    fn set_selected(&mut self, value: i32) -> Result<()> {
        let mut call = button::Setter::new(value);
        while let Some(request) = call.advance(&mut self.state).map_err(anyhow::Error::msg)? {
            let reply = match request {
                button::Request::Changed => {
                    self.shown = self.state != 0;
                    if !self.shown {
                        self.popup = None;
                    }
                    button::Reply::Done
                }
                button::Request::Invalidate | button::Request::InvalidateParent(1) => {
                    self.invalidations = self.invalidations.wrapping_add(1);
                    button::Reply::Done
                }
                // The actual replacement sidebar is the stable parent of this
                // button. Both native lookups still run through the setter.
                button::Request::Parent => button::Reply::Parent(Some(1)),
                _ => anyhow::bail!("needs button parent differs"),
            };
            call.acknowledge(&request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    fn toggle_popup(&mut self, index: usize) -> Result<()> {
        ensure!(
            self.shown && self.person.is_some() && index < MOTIVES.len(),
            "motive widget is unavailable"
        );
        //51b7f6: closed -> open, same content -> close, different -> replace.
        self.popup = if self.popup == Some(index as u8) {
            None
        } else {
            Some(index as u8)
        };
        self.invalidations = self.invalidations.wrapping_add(1);
        Ok(())
    }
}

/// Read the actually selected live object. Cache indices and neighborhood IDs
/// are distinct. A stale imported object number never substitutes for identity.
fn selected_person(session: &Session) -> Result<Option<Person>> {
    let Some(id) = session
        .simulation
        .native_selected_person()
        .map_err(anyhow::Error::msg)?
    else {
        return Ok(None);
    };
    let world = &session.simulation.vm.world;
    let object = world
        .objects
        .get(&id)
        .context("selected live person absent")?;
    let definition = object
        .definition
        .as_ref()
        .context("selected live definition absent")?;
    let data = object
        .person
        .as_ref()
        .context("selected object is not a person")?;
    let kind = *data.get(&108).context("selected person species absent")?;
    ensure!(kind <= 1, "replacement pet needs widgets are not connected");
    let (cache_index, _) = world
        .image_cache
        .entries
        .iter()
        .enumerate()
        .find(|(_, e)| e.guid == definition.guid)
        .context("selected person's image-cache identity absent")?;
    let person = world
        .person
        .as_ref()
        .context("live person registry absent")?;
    let family = person
        .families
        .get(&person.controlled_family)
        .context("controlled family absent")?;
    let Some(&neighbor) = family.iter().find(|n| {
        person
            .neighbors
            .get(n)
            .is_some_and(|n| n.guid == definition.guid)
    }) else {
        return Ok(None);
    };
    let raw = object
        .motives
        .context("selected person's live motives absent")?;
    let values = std::array::from_fn(|i| raw[MOTIVES[i].1]);
    ensure!(
        values.iter().all(|v| f32::from_bits(*v).is_finite()),
        "non-finite live motive"
    );
    let name = session
        .house
        .household
        .as_ref()
        .and_then(|h| h.residents.iter().find(|r| r.guid == definition.guid))
        .map(|r| r.name.clone())
        .unwrap_or_else(|| format!("Sim {neighbor}"));
    Ok(Some(Person {
        object: id,
        guid: definition.guid,
        cache_index: cache_index as u32,
        neighbor,
        name,
        values,
    }))
}
impl Client {
    pub fn live_panel(&self) -> Option<&Panel> {
        self.live_panel.as_ref()
    }
    /// Explicit frontend construction. Restoring a historical client does not
    /// fabricate this owner or retroactively acknowledge an original UI wait.
    pub fn bind_live_panel(&mut self, session: &Session) -> Result<()> {
        ensure!(
            self.live_panel.is_none() && !self.pending(),
            "live panel already bound or UI pending"
        );
        ensure!(
            self.buttons.as_ref().is_some_and(|b| b.initialized),
            "live panel requires resident widgets"
        );
        self.validate(session)?;
        let person = selected_person(session)?;
        let mut panel = Panel {
            tick: session.simulation.vm.tick,
            person,
            state: 0,
            shown: false,
            popup: None,
            invalidations: 0,
        };
        //518665 selects Needs at construction. The replacement layout owns only
        // this category today; it has no invisible fake buttons for the others.
        panel.set_selected(i32::from(panel.person.is_some()))?;
        panel.validate(session)?;
        self.live_panel = Some(panel);
        Ok(())
    }
    pub fn refresh_live_panel(&mut self, session: &Session) -> Result<()> {
        if self.live_panel.is_none() {
            return Ok(());
        }
        ensure!(
            !self.pending(),
            "cannot refresh live widgets during presentation work"
        );
        let person = selected_person(session)?;
        let panel = self.live_panel.as_mut().unwrap();
        panel.set_selected(i32::from(person.is_some()))?;
        panel.person = person;
        panel.tick = session.simulation.vm.tick;
        panel.validate(session)
    }
    pub fn select_needs(&mut self, session: &Session) -> Result<()> {
        self.refresh_live_panel(session)?;
        let panel = self.live_panel.as_mut().context("needs widget absent")?;
        panel.set_selected(i32::from(panel.person.is_some()))
    }
    pub fn toggle_motive_details(&mut self, session: &Session, index: usize) -> Result<()> {
        self.refresh_live_panel(session)?;
        self.live_panel
            .as_mut()
            .context("needs widget absent")?
            .toggle_popup(index)
    }
    pub fn close_motive_details(&mut self) -> Result<()> {
        let panel = self.live_panel.as_mut().context("needs widget absent")?;
        panel.popup = None;
        panel.invalidations = panel.invalidations.wrapping_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel() -> Panel {
        Panel {
            tick: 0,
            person: Some(Person {
                object: 1,
                guid: 42,
                cache_index: 2,
                neighbor: 3,
                name: "Test".into(),
                values: [0; 8],
            }),
            state: 0,
            shown: false,
            popup: None,
            invalidations: 0,
        }
    }
    #[test]
    fn actual_widget_selection_and_popup_identity_survive_serialization() -> Result<()> {
        let mut p = panel();
        p.set_selected(1)?;
        ensure!(p.shown() && p.invalidations == 2);
        p.toggle_popup(0)?;
        p.set_selected(1)?; // Native equal-state setter does not close the popup.
        ensure!(p.popup() == Some(0) && p.invalidations == 3);
        p = serde_json::from_slice(&serde_json::to_vec(&p)?)?;
        p.toggle_popup(1)?;
        ensure!(p.popup() == Some(1));
        p.toggle_popup(1)?;
        ensure!(p.popup().is_none());
        p.toggle_popup(0)?;
        p.set_selected(0)?;
        ensure!(!p.shown() && p.popup().is_none());
        let before = p.clone();
        ensure!(p.toggle_popup(0).is_err() && p == before);
        Ok(())
    }
}
