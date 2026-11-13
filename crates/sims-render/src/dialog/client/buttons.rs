//! Original portrait lookup shared by actual resident buttons, 47689c's
//! highlight and 4764fe's portrait observation. UI layout is the replacement
//! frontend's responsibility; image references and synchronous calls persist.
use super::*;
use crate::portrait::retrieval::{Progress as Retrieved, Retrieval};
use sims_vm::{
    ui_effect,
    ui_owner::wait::{Condition, Observation},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    pub neighbor: i16,
    pub guid: u32,
    pub image: Option<u32>,
    pub highlighted: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Resident(i16),
    Control(i32),
    Unpictured,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buttons {
    pub items: Vec<Button>,
    /// 6fe9a4: last activated button, not hover or selected person. Native
    /// 5e4478/5e4563 publish before dispatch; 5e39ac clears on destruction.
    pub activated: Option<Key>,
    pub initialized: bool,
    /// The replacement HUD subscribes to resolved portrait changes. Historical
    /// window-tree checkpoints retain their explicitly bound image wrappers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    follow_images: bool,
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use sims_vm::ui_owner::wait::{Poll, Reply, Request, State};

    fn buttons() -> Buttons {
        Buttons {
            items: vec![Button {
                neighbor: 1,
                guid: 42,
                image: Some(7),
                highlighted: true,
            }],
            activated: None,
            initialized: true,
            follow_images: true,
        }
    }

    #[test]
    fn button_focus_observes_activation_not_selection_or_requested_resource() {
        let mut buttons = buttons();
        let condition = Condition::Focus(4500);
        assert_eq!(
            buttons.observation(&condition),
            Some(Observation::Focus(None))
        );
        buttons.activated = Some(Key::Control(2010));
        assert_eq!(
            buttons.observation(&condition),
            Some(Observation::Focus(Some(2010)))
        );
        buttons.activated = Some(Key::Resident(1));
        assert_eq!(
            buttons.observation(&condition),
            Some(Observation::Focus(Some(-1)))
        );
        buttons.activated = Some(Key::Unpictured);
        assert_eq!(
            buttons.observation(&condition),
            Some(Observation::Focus(None))
        );
        assert_eq!(buttons.observation(&Condition::Control(4500)), None);
        buttons.activated = Some(Key::Control(4500));
        let restored: Buttons =
            serde_json::from_slice(&serde_json::to_vec(&buttons).unwrap()).unwrap();
        assert_eq!(restored, buttons);
        assert_eq!(
            restored.observation(&condition),
            Some(Observation::Focus(Some(4500)))
        );
    }

    #[test]
    fn button_focus_wait_requires_the_actual_matching_activation() {
        for (activation, matched) in [
            (None, false),
            (Some(Key::Resident(1)), false),
            (Some(Key::Unpictured), false),
            (Some(Key::Control(2010)), false),
            (Some(Key::Control(4500)), true),
        ] {
            let mut buttons = buttons();
            buttons.activated = activation;
            let before = buttons.clone();
            let mut poll = Poll::begin(Some(2), buttons.focused_resource()).unwrap();
            let mut state = State::default();
            let publish = poll.request().unwrap().clone();
            poll.acknowledge(&mut state, &publish, Reply::Done).unwrap();
            poll.acknowledge(
                &mut state,
                &Request::Query(2),
                Reply::Query(Some([4, 4500])),
            )
            .unwrap();
            let condition = Condition::Focus(4500);
            poll.acknowledge(
                &mut state,
                &Request::Observe(condition.clone()),
                Reply::Observed(buttons.observation(&condition).unwrap()),
            )
            .unwrap();
            assert_eq!(poll.request() == Some(&Request::Got(2)), matched);
            assert_eq!(buttons, before);
        }
    }
}

impl Buttons {
    pub fn image(&self, neighbor: i16) -> Option<u32> {
        self.items
            .iter()
            .find(|b| b.neighbor == neighbor)
            .and_then(|b| b.image)
    }
    pub fn highlighted(&self, neighbor: i16) -> bool {
        self.items
            .iter()
            .any(|b| b.neighbor == neighbor && b.highlighted)
    }
    fn bitmap(&self, key: Key) -> Option<(u32, i32)> {
        match key {
            // Procedural cTSBuffer portraits retain constructor +30=-1.
            Key::Resident(n) => self.image(n).map(|id| (id, -1)),
            // Separate namespace from image-cache handles (bounded to1M).
            // These are native-identified replacement controls, not addresses.
            Key::Control(id) => Some((0x8000_0000 | id as u32, id)),
            Key::Unpictured => None,
        }
    }
    pub fn focused_resource(&self) -> Option<i32> {
        self.activated.and_then(|k| self.bitmap(k)).map(|(_, r)| r)
    }
    pub fn observation(&self, condition: &Condition) -> Option<Observation> {
        match condition {
            Condition::Focus(_) => Some(Observation::Focus(self.focused_resource())),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Purpose {
    Bind(usize),
    Highlight {
        thread: sims_vm::ThreadId,
        request: ui_effect::Request,
    },
    Observe {
        captured: Option<Key>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lookup {
    tick: u64,
    neighbor: i16,
    purpose: Purpose,
    inputs: Inputs,
    retrieval: Option<Retrieval>,
}
impl Lookup {
    pub(super) fn validate(&self, s: &Session, images: &Images) -> Result<()> {
        ensure!(
            self.tick == s.simulation.vm.tick
                && s.host.presentation.ui_portrait_tick == Some(self.tick),
            "UI portrait lost its clock owner"
        );
        self.inputs.view.build().map_err(anyhow::Error::msg)?;
        if let Some(call) = &self.retrieval {
            call.validate(s, images)?;
        }
        self.validate_source(s)
    }
    fn validate_source(&self, s: &Session) -> Result<()> {
        match &self.purpose {
            Purpose::Highlight { thread, request } => {
                let Some(t) = s.simulation.vm.threads.get(thread) else {
                    anyhow::bail!("portrait highlight thread absent")
                };
                ensure!(
                    matches!(&t.status, sims_vm::Status::AwaitingUiEffect(c) if c.request() == *request),
                    "portrait highlight lost its original request"
                );
                ensure!(
                    request.kind == ui_effect::Kind::Portrait && request.value == self.neighbor,
                    "portrait highlight arguments differ"
                );
            }
            Purpose::Observe { .. } => ensure!(
                s.pending_ui_observation() == Some(Condition::Portrait(self.neighbor)),
                "portrait observation lost its original poll"
            ),
            Purpose::Bind(_) => ensure!(
                !s.ui_wait_pending()
                    && !s
                        .simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_))),
                "resident construction overlaps original UI work"
            ),
        }
        Ok(())
    }
}
impl Client {
    pub fn follow_resident_portraits(&mut self) -> Result<()> {
        ensure!(
            !self.pending(),
            "portrait subscription overlaps a client call"
        );
        let buttons = self.buttons.as_mut().context("resident buttons absent")?;
        ensure!(buttons.initialized, "resident buttons not initialized");
        buttons.follow_images = true;
        Ok(())
    }
    pub fn buttons(&self) -> Option<&Buttons> {
        self.buttons.as_ref()
    }
    pub fn ui_lookup_pending(&self) -> bool {
        self.lookup.is_some()
    }
    pub fn activate_button(&mut self, key: Key) -> Result<()> {
        ensure!(
            !self.pending(),
            "button activation overlaps presentation work"
        );
        let Some(buttons) = &mut self.buttons else {
            return Ok(());
        };
        ensure!(buttons.initialized, "resident buttons still constructing");
        if let Key::Resident(n) = key {
            ensure!(
                buttons.items.iter().any(|b| b.neighbor == n),
                "activated resident button absent"
            );
        }
        if let Key::Control(id) = key {
            ensure!((0..0x10000).contains(&id), "invalid control resource");
        }
        buttons.activated = Some(key);
        Ok(())
    }
    pub(super) fn validate_buttons(&self, s: &Session) -> Result<()> {
        ensure!(
            self.lookup.is_some() == s.ui_portrait_pending(),
            "UI portrait lost its client owner"
        );
        if let Some(buttons) = &self.buttons {
            let cache = &s.simulation.vm.world.image_cache;
            let mut count = vec![0u32; cache.refs.len()];
            for id in cache
                .entries
                .iter()
                .flat_map(|e| e.images)
                .flatten()
                .chain(buttons.items.iter().filter_map(|b| b.image))
            {
                *count
                    .get_mut(id.wrapping_sub(1) as usize)
                    .context("resident image owner absent")? += 1;
                self.images.get(id)?;
            }
            ensure!(
                count.iter().zip(&cache.refs).all(|(n, r)| n <= r),
                "resident image ownership exceeds references"
            );
            let mut neighbors = std::collections::BTreeSet::new();
            for b in &buttons.items {
                ensure!(
                    neighbors.insert(b.neighbor) && b.neighbor > 0,
                    "duplicate/invalid resident button"
                );
                ensure!(
                    s.house.household.as_ref().is_some_and(|h| h
                        .residents
                        .iter()
                        .any(|r| r.neighbor == b.neighbor && r.guid == b.guid)),
                    "resident button belongs to another household"
                );
            }
            if let Some(Key::Resident(n)) = buttons.activated {
                ensure!(
                    neighbors.contains(&n),
                    "activated resident button was destroyed"
                );
            }
            if !buttons.initialized {
                ensure!(
                    self.lookup
                        .as_ref()
                        .is_some_and(|l| matches!(l.purpose, Purpose::Bind(_))),
                    "resident construction lost its continuation"
                );
            }
            if let Some(Lookup {
                neighbor,
                purpose: Purpose::Bind(index),
                ..
            }) = &self.lookup
            {
                ensure!(
                    !buttons.initialized
                        && buttons
                            .items
                            .get(*index)
                            .is_some_and(|b| b.neighbor == *neighbor),
                    "resident construction slot differs"
                );
            }
            if let Some(Lookup {
                purpose: Purpose::Observe { captured },
                ..
            }) = &self.lookup
            {
                ensure!(
                    buttons.initialized && *captured == buttons.activated,
                    "button activation changed during synchronous portrait observation"
                );
            }
            if let Some(Key::Control(id)) = buttons.activated {
                ensure!((0..0x10000).contains(&id), "invalid saved control resource");
            }
        } else {
            ensure!(self.lookup.is_none(), "UI lookup has no real buttons");
        }
        Ok(())
    }
    fn begin_lookup(
        &mut self,
        s: &mut Session,
        neighbor: i16,
        purpose: Purpose,
        inputs: Inputs,
    ) -> Result<()> {
        ensure!(!self.pending(), "UI portrait overlaps another client call");
        let skip = matches!(purpose, Purpose::Observe { captured: None });
        let index = if skip {
            None
        } else {
            ui_effect::neighbor_guid(&s.simulation.vm.world, neighbor)
                .map_err(anyhow::Error::msg)?
                .map(|guid| {
                    s.simulation
                        .vm
                        .world
                        .image_cache
                        .entries
                        .iter()
                        .position(|e| e.guid == guid)
                        .map_or(u32::MAX, |n| n as u32)
                })
        };
        inputs.view.build().map_err(anyhow::Error::msg)?;
        let lookup = Lookup {
            tick: s.simulation.vm.tick,
            neighbor,
            purpose,
            inputs,
            retrieval: index.map(|index| {
                Retrieval::new(
                    s.simulation.vm.tick,
                    index,
                    None,
                    Some(self.background),
                    Some(self.thumbnail),
                    u32::from(self.settings.key),
                )
            }),
        };
        lookup.validate_source(s)?;
        s.begin_ui_portrait()?;
        self.lookup = Some(lookup);
        self.validate(s)
    }
    /// Explicit real-widget construction. Old saves keep their absent binding;
    /// restore never guesses an activated button or silently builds this owner.
    pub fn bind_resident_buttons(&mut self, s: &mut Session, inputs: Inputs) -> Result<()> {
        ensure!(
            self.buttons.is_none() && !self.pending(),
            "resident buttons already bound"
        );
        self.validate(s)?;
        let items: Vec<_> = s
            .house
            .household
            .as_ref()
            .context("button household absent")?
            .residents
            .iter()
            .map(|r| Button {
                neighbor: r.neighbor,
                guid: r.guid,
                image: None,
                highlighted: false,
            })
            .collect();
        let first = items.first().map(|b| b.neighbor);
        self.buttons = Some(Buttons {
            items,
            activated: None,
            initialized: first.is_none(),
            follow_images: false,
        });
        if let Some(n) = first {
            if let Err(error) = self.begin_lookup(s, n, Purpose::Bind(0), inputs) {
                if self.lookup.is_none() {
                    self.buttons = None;
                }
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn begin_portrait_highlight(
        &mut self,
        s: &mut Session,
        thread: sims_vm::ThreadId,
        request: ui_effect::Request,
        inputs: Inputs,
    ) -> Result<()> {
        ensure!(
            self.buttons.as_ref().is_some_and(|b| b.initialized),
            "portrait buttons not bound"
        );
        self.begin_lookup(
            s,
            request.value,
            Purpose::Highlight { thread, request },
            inputs,
        )
    }
    pub fn begin_portrait_observation(
        &mut self,
        s: &mut Session,
        neighbor: i16,
        inputs: Inputs,
    ) -> Result<()> {
        let b = self
            .buttons
            .as_ref()
            .context("portrait buttons not bound")?;
        ensure!(b.initialized, "portrait buttons not constructed");
        self.begin_lookup(
            s,
            neighbor,
            Purpose::Observe {
                captured: b.activated,
            },
            inputs,
        )
    }
    /// Drives the same original getter/regeneration and temporary-person host
    /// used by dialogs. Publication occurs only after that getter returns.
    pub fn advance_ui_lookup(&mut self, s: &mut Session, budget: usize) -> Result<bool> {
        ensure!(budget > 0, "UI portrait budget is zero");
        self.collect(s)?;
        self.validate(s)?;
        let mut remaining = budget;
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        for _ in 0..1000 {
            s.host.update_contexts(&mut s.simulation)?;
            let lookup = self.lookup.as_mut().context("UI portrait call absent")?;
            let inputs = lookup.inputs;
            let progress = if let Some(call) = &mut lookup.retrieval {
                call.advance(
                    &mut cache::Session {
                        session: s,
                        view: inputs.view,
                        source_key: None,
                    },
                    &mut self.catalog,
                    &mut self.images,
                    &self.settings,
                    &mut remaining,
                    &mut step,
                )?
            } else {
                Retrieved::Complete(false)
            };
            if let Retrieved::Temporary(service) = progress {
                self.service_temporary(s, inputs, service)?;
            }
            let serviced = s.host.service(&mut s.simulation, &mut s.program)?;
            self.collect(s)?;
            if matches!(progress, Retrieved::Complete(_)) {
                return self.finish_lookup(s);
            }
            if remaining == 0 {
                return Ok(false);
            }
            if progress == Retrieved::Yielded && serviced == 0 {
                anyhow::bail!("UI portrait yielded without a serviced dependency");
            }
        }
        anyhow::bail!("UI portrait service budget exceeded")
    }
    fn finish_lookup(&mut self, s: &mut Session) -> Result<bool> {
        self.validate(s)?;
        let lookup = self.lookup.take().unwrap();
        let (found, mut image) = lookup
            .retrieval
            .map_or(Ok((false, None)), Retrieval::finish)?;
        let mut next = None;
        let mut observation = None;
        let mut effect = None;
        // A regenerated portrait is a new image wrapper. Publish that actual
        // image to the subscribed widget before highlight/focus compares it;
        // keep the button's logical identity and last activation unchanged.
        // This is a replacement cache subscriber, not an acknowledgement of a
        // missing native cWinLive callback. It does not request another picture,
        // construct a person, advance scripts, or consume random numbers.
        if found
            && !matches!(lookup.purpose, Purpose::Bind(_))
            && let Some(buttons) = self.buttons.as_mut().filter(|b| b.follow_images)
            && let Some(button) = buttons
                .items
                .iter_mut()
                .find(|b| b.neighbor == lookup.neighbor)
        {
            ensure!(
                ui_effect::neighbor_guid(&s.simulation.vm.world, lookup.neighbor)
                    .map_err(anyhow::Error::msg)?
                    == Some(button.guid),
                "portrait subscriber identity changed"
            );
            s.simulation
                .vm
                .world
                .image_cache
                .assign_image(&mut button.image, image, &mut |e, c| {
                    self.images.event(e, c)
                })
                .map_err(anyhow::Error::msg)?;
        }
        match lookup.purpose {
            Purpose::Bind(index) => {
                let buttons = self.buttons.as_mut().unwrap();
                let b = buttons
                    .items
                    .get_mut(index)
                    .context("resident construction index absent")?;
                ensure!(
                    b.neighbor == lookup.neighbor,
                    "resident construction neighbor changed"
                );
                if found {
                    s.simulation
                        .vm
                        .world
                        .image_cache
                        .assign_image(&mut b.image, image, &mut |e, c| self.images.event(e, c))
                        .map_err(anyhow::Error::msg)?;
                }
                if let Some(b) = buttons.items.get(index + 1) {
                    next = Some((index + 1, b.neighbor));
                } else {
                    buttons.initialized = true;
                }
            }
            Purpose::Highlight { thread, request } => {
                if found {
                    // Native lookup compares the actual shared image object,
                    // not a neighbor number or the currently selected Sim.
                    if let Some(b) = self
                        .buttons
                        .as_mut()
                        .unwrap()
                        .items
                        .iter_mut()
                        .find(|b| b.image == image)
                    {
                        b.highlighted = request.enabled;
                    }
                }
                effect = Some((thread, request));
            }
            Purpose::Observe { captured } => {
                let bitmap = captured.and_then(|k| self.buttons.as_ref().unwrap().bitmap(k));
                observation = Some(Observation::Portrait {
                    focus_present: captured.is_some(),
                    focus_image: bitmap.map_or(0, |(id, _)| id),
                    resolved: if found { image } else { None },
                });
            }
        }
        // 47689c releases after widget publication; 4764fe releases before its
        // got-wait callback. Button bindings retained their own wrapper above.
        self.images
            .release(&mut s.simulation.vm.world.image_cache, &mut image)?;
        s.finish_ui_portrait()?;
        if let Some((thread, request)) = effect {
            s.simulation
                .vm
                .complete_ui_effect(&s.program, thread, request)
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(observation) = observation {
            s.acknowledge_ui_wait(&Condition::Portrait(lookup.neighbor), observation)?;
        }
        if let Some((index, neighbor)) = next {
            self.begin_lookup(s, neighbor, Purpose::Bind(index), lookup.inputs)?;
            return Ok(false);
        }
        self.validate(s)?;
        Ok(true)
    }
}
