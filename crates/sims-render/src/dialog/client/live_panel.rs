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
mod details;
pub use details::{Category, Data, Detail, Identity, Job, PERSONALITY, Popup, Relationship, SKILLS, Texts};
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
    #[serde(default)]
    pub details: Data,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Panel {
    tick: u64,
    person: Option<Person>,
    state: i32,
    shown: bool,
    popup: Option<Popup>,
    #[serde(default)]
    category: Category,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    texts: Option<Texts>,
    /// Replacement widget invalidations. Iced rebuilds the affected view after
    /// the owning input/update; the native setter's equal-state branch is quiet.
    invalidations: u64,
}
impl Panel {
    pub fn person(&self) -> Option<&Person> {
        self.person.as_ref()
    }
    pub fn selected(&self) -> bool {
        self.category_selected(Category::Needs)
    }
    pub fn category(&self) -> Category {
        self.category
    }
    pub fn category_selected(&self, category: Category) -> bool {
        self.state != 0 && self.category == category
    }
    pub fn texts(&self) -> Option<&Texts> {
        self.texts.as_ref()
    }
    pub fn retained_popup(&self) -> Option<Popup> {
        self.popup
    }
    pub fn shown(&self) -> bool {
        self.shown
    }
    pub fn popup(&self) -> Option<usize> {
        match self.popup { Some(Popup::Motive(index)) => Some(usize::from(index)), _ => None }
    }
    pub fn detail_available(&self, detail: Detail) -> bool {
        let Some(person) = &self.person else { return false };
        self.shown && self.category == detail.category() && self.texts.is_some() && match detail {
            Detail::Personality(index) => person.details.personality.is_some() && usize::from(index) < PERSONALITY.len(),
            Detail::Relationship { neighbor, guid, cache_index } => person.details.relationships.iter()
                .any(|row| row.neighbor == neighbor && row.guid == guid && row.cache_index == cache_index),
            Detail::Job => person.details.job.is_some(),
            Detail::Skill(index) => person.details.skills.is_some() && usize::from(index) < SKILLS.len(),
        }
    }
    pub fn observation(&self, condition: &Condition) -> Option<Observation> {
        match condition {
            Condition::Control(resource) if Category::ALL.iter().any(|category| category.control() == *resource) => {
                Some(Observation::Control {
                    image: Some(*resource),
                    state: u32::from(self.shown && self.category.control() == *resource),
                })
            }
            Condition::Panel(_) | Condition::PanelAbsent => Some(Observation::Panel(
                self.popup.map(|popup| popup.name().to_string()),
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
            self.popup.is_none_or(|popup| match popup {
                Popup::Motive(index) => index < 8 && self.shown && self.category == Category::Needs,
                Popup::Detail(detail) => self.detail_available(detail),
            }),
            "motive popup lost its widget"
        );
        if let Some(texts) = &self.texts { texts.validate()?; }
        if let Some(person) = &self.person {
            ensure!(selected_person(session)?.as_ref().map(Person::identity) == Some(person.identity()),
                "displayed live person selection changed");
            for relationship in &person.details.relationships {
                ensure!(session.simulation.vm.world.image_cache.entries.get(relationship.cache_index as usize)
                    .is_some_and(|entry| entry.guid == relationship.guid)
                    && session.simulation.vm.world.person.as_ref().and_then(|registry| registry.neighbors.get(&relationship.neighbor))
                        .is_some_and(|neighbor| neighbor.guid == relationship.guid)
                    && (-100..=100).contains(&relationship.daily) && (-100..=100).contains(&relationship.lifetime),
                    "displayed relationship identity or value changed");
            }
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
            self.shown && self.category == Category::Needs && self.person.is_some() && index < MOTIVES.len(),
            "motive widget is unavailable"
        );
        //51b7f6: closed -> open, same content -> close, different -> replace.
        self.popup = if self.popup == Some(Popup::Motive(index as u8)) {
            None
        } else {
            Some(Popup::Motive(index as u8))
        };
        self.invalidations = self.invalidations.wrapping_add(1);
        Ok(())
    }
    fn select_category(&mut self, category: Category) {
        if self.category != category {
            self.popup = None;
            self.category = category;
            self.invalidations = self.invalidations.wrapping_add(1);
        }
    }
    fn toggle_detail(&mut self, identity: Identity, detail: Detail) -> bool {
        if self.person.as_ref().map(Person::identity) != Some(identity) || !self.detail_available(detail) { return false; }
        let popup = Some(Popup::Detail(detail));
        self.popup = if self.popup == popup { None } else { popup };
        self.invalidations = self.invalidations.wrapping_add(1);
        true
    }
    fn refresh_person(&mut self, mut person: Option<Person>, tick: u64) -> Result<()> {
        if self.person.as_ref().map(Person::identity) != person.as_ref().map(Person::identity) {
            self.popup = None;
        } else if let Some(person) = &mut person {
            for relationship in &mut person.details.relationships {
                relationship.highlighted = self.person.as_ref().is_some_and(|previous|
                    previous.details.relationships.iter().any(|row| row.neighbor == relationship.neighbor
                        && row.guid == relationship.guid && row.cache_index == relationship.cache_index && row.highlighted));
            }
        }
        self.set_selected(i32::from(person.is_some()))?;
        self.person = person;
        if self.popup.is_some_and(|popup| matches!(popup, Popup::Detail(detail) if !self.detail_available(detail))) {
            self.popup = None;
        }
        self.tick = tick;
        Ok(())
    }
    fn highlight_relationship(&mut self, neighbor: i16, guid: u32, cache_index: u32, enabled: bool) -> Result<()> {
        let person = self.person.as_mut().context("relationship highlight has no selected person")?;
        let relationship = person.details.relationships.iter_mut()
            .find(|row| row.neighbor == neighbor && row.guid == guid && row.cache_index == cache_index)
            .context("relationship highlight requires its actual selected-person target widget")?;
        if relationship.highlighted != enabled {
            relationship.highlighted = enabled;
            self.invalidations = self.invalidations.wrapping_add(1);
        }
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
        details: details::read(session, data, neighbor)?,
    }))
}
pub fn selected_identity(session: &Session) -> Result<Option<Identity>> {
    Ok(selected_person(session)?.map(|person| person.identity()))
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
            category: Category::Needs,
            texts: None,
            invalidations: 0,
        };
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
        panel.refresh_person(person, session.simulation.vm.tick)?;
        panel.validate(session)
    }
    pub fn select_needs(&mut self, session: &Session) -> Result<()> {
        self.refresh_live_panel(session)?;
        let panel = self.live_panel.as_mut().context("needs widget absent")?;
        panel.select_category(Category::Needs);
        panel.set_selected(i32::from(panel.person.is_some()))
    }
    pub fn bind_live_panel_texts(&mut self, texts: Texts) -> Result<()> {
        texts.validate()?;
        self.live_panel.as_mut().context("live panel absent")?.texts = Some(texts);
        Ok(())
    }
    pub fn select_person_category(&mut self, session: &Session, identity: Identity, category: Category) -> Result<bool> {
        self.refresh_live_panel(session)?;
        let panel = self.live_panel.as_mut().context("live panel absent")?;
        if panel.person.as_ref().map(Person::identity) != Some(identity) { return Ok(false); }
        panel.select_category(category);
        Ok(true)
    }
    pub fn toggle_person_details(&mut self, session: &Session, identity: Identity, detail: Detail) -> Result<bool> {
        self.refresh_live_panel(session)?;
        let panel = self.live_panel.as_mut().context("live panel absent")?;
        Ok(panel.toggle_detail(identity, detail))
    }
    pub fn close_person_details(&mut self, session: &Session, identity: Identity, expected: Popup) -> Result<bool> {
        self.refresh_live_panel(session)?;
        let panel = self.live_panel.as_mut().context("live panel absent")?;
        if panel.person.as_ref().map(Person::identity) != Some(identity) || panel.popup != Some(expected) { return Ok(false); }
        panel.popup = None;
        panel.invalidations = panel.invalidations.wrapping_add(1);
        Ok(true)
    }
    pub fn highlight_live_relationship(&mut self, session: &Session, request: sims_vm::ui_effect::Request) -> Result<()> {
        ensure!(request.kind == sims_vm::ui_effect::Kind::Relationship, "non-relationship highlight request");
        let world = &session.simulation.vm.world;
        let Some(guid) = sims_vm::ui_effect::neighbor_guid(world, request.value).map_err(anyhow::Error::msg)? else { return Ok(()) };
        let registry = world.person.as_ref().context("relationship neighborhood absent")?;
        let neighbor = registry.neighbor_slots.as_ref().and_then(|slots| slots.get(request.value as usize - 1))
            .copied().flatten().context("relationship target neighbor slot absent")?;
        let cache_index = world.image_cache.entries.iter().position(|entry| entry.guid == guid)
            .context("relationship highlight target cache identity absent")?;
        self.refresh_live_panel(session)?;
        self.live_panel.as_mut().context("relationship panel owner absent")?
            .highlight_relationship(neighbor, guid, cache_index as u32, request.enabled)
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
                details: Data::default(),
            }),
            state: 0,
            shown: false,
            popup: None,
            category: Category::Needs,
            texts: None,
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
    fn details_panel() -> Result<Panel> {
        let mut panel = panel();
        panel.texts = Some(Texts { personality: vec!["trait".into(); 36], relationships: vec!["relation".into(); 3],
            skills: vec!["skill".into(); 7], details: vec!["detail".into(); 24] });
        let person = panel.person.as_mut().unwrap();
        person.details.personality = Some([100, 200, 300, 400, 500]);
        person.details.skills = Some([100, 200, 300, 400, 500, 600]);
        person.details.relationships.push(Relationship { neighbor: 4, guid: 43, cache_index: 3,
            name: "Other".into(), daily: 12, lifetime: -20, flags: [false; 2], highlighted: false });
        person.details.job = Some(Job { career: 1, level: 0, title: "Actual career title".into(),
            description: "Description".into(), start: 9, end: 17, next_skills: Some([200; 6]) });
        panel.set_selected(1)?;
        Ok(panel)
    }
    #[test]
    fn tutorial_panel_popup_ownership_roundtrips_and_never_expires_on_refresh() -> Result<()> {
        for detail in [Detail::Personality(0), Detail::Relationship { neighbor: 4, guid: 43, cache_index: 3 }, Detail::Job, Detail::Skill(0)] {
            let mut panel = details_panel()?;
            panel.select_category(detail.category());
            let identity = panel.person().unwrap().identity();
            ensure!(panel.observation(&Condition::Panel(detail.name().into())) == Some(Observation::Panel(None)));
            ensure!(panel.toggle_detail(identity, detail));
            panel = serde_json::from_slice(&serde_json::to_vec(&panel)?)?;
            let person = panel.person.clone();
            for tick in 1..=25 { panel.refresh_person(person.clone(), tick)?; }
            ensure!(panel.category() == detail.category());
            ensure!(panel.observation(&Condition::PanelAbsent) == Some(Observation::Panel(Some(detail.name().into()))));
            ensure!(panel.toggle_detail(identity, detail) && panel.retained_popup().is_none());
        }
        Ok(())
    }
    #[test]
    fn tutorial_panel_actual_categories_and_stale_person_gate_popup_state() -> Result<()> {
        let mut panel = details_panel()?;
        let identity = panel.person().unwrap().identity();
        panel.select_category(Category::Personality);
        for category in Category::ALL {
            ensure!(panel.observation(&Condition::Control(category.control())) == Some(Observation::Control {
                image: Some(category.control()), state: u32::from(category == Category::Personality) }));
        }
        ensure!(panel.toggle_detail(identity, Detail::Personality(0)));
        ensure!(panel.toggle_detail(identity, Detail::Personality(1)));
        ensure!(panel.retained_popup() == Some(Popup::Detail(Detail::Personality(1))));
        let mut replacement = panel.person.clone().unwrap();
        replacement.object = 9;
        replacement.guid = 99;
        panel.refresh_person(Some(replacement), 10)?;
        ensure!(panel.retained_popup().is_none() && !panel.toggle_detail(identity, Detail::Personality(1)));
        panel.refresh_person(None, 11)?;
        ensure!(!panel.shown() && panel.observation(&Condition::PanelAbsent) == Some(Observation::Panel(None)));
        Ok(())
    }
    #[test]
    fn tutorial_panel_unknown_detail_data_does_not_publish_popup_names() -> Result<()> {
        let mut panel = details_panel()?;
        let identity = panel.person().unwrap().identity();
        panel.select_category(Category::Job);
        panel.person.as_mut().unwrap().details.job = None;
        ensure!(!panel.toggle_detail(identity, Detail::Job));
        ensure!(!panel.toggle_detail(identity, Detail::Skill(6)));
        ensure!(panel.observation(&Condition::Panel("job".into())) == Some(Observation::Panel(None)));
        ensure!(panel.observation(&Condition::Focus(4503)).is_none());
        Ok(())
    }
    #[test]
    fn tutorial_panel_relationship_highlight_tracks_real_target_and_selected_identity() -> Result<()> {
        let mut panel = details_panel()?;
        let before = panel.clone();
        ensure!(panel.highlight_relationship(4, 999, 3, true).is_err() && panel == before);
        ensure!(panel.highlight_relationship(4, 43, 99, true).is_err() && panel == before);
        panel.highlight_relationship(4, 43, 3, true)?;
        panel = serde_json::from_slice(&serde_json::to_vec(&panel)?)?;
        let mut next = before.person.clone();
        next.as_mut().unwrap().details.relationships[0].daily = 20;
        panel.refresh_person(next, 1)?;
        ensure!(panel.person().unwrap().details.relationships[0].highlighted);
        panel.highlight_relationship(4, 43, 3, false)?;
        ensure!(!panel.person().unwrap().details.relationships[0].highlighted);
        panel.highlight_relationship(4, 43, 3, true)?;
        let mut changed = before.person.clone();
        changed.as_mut().unwrap().guid = 999;
        panel.refresh_person(changed, 2)?;
        ensure!(!panel.person().unwrap().details.relationships[0].highlighted);
        Ok(())
    }
}
