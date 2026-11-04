//! Saved replacement widgets for 50a8ec -> 53ab6d -> 53b536.
//!
//! Native callers and setters run unchanged against these owners. Changed
//! publishes Iced's selected style; invalidation marks its real widget/parent.
//! Native bitmap allocation and Windows painting are not part of this backend.
//! Panel visibility and the remaining ViewControl services stay explicit.
use super::{Device, Saved, mode};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::{
    Session,
    publication::{PanelAccess, PanelOwner},
};
use sims_vm::{
    interface::{
        panel_dispatch as dispatch, panel_switch,
        state::State,
        view_control::{self as control, buttons},
    },
    ui_effect::widgets::{button, timers::Entry, visibility},
};
use std::collections::{BTreeMap, BTreeSet};
mod access;
pub mod catalog;
pub mod presentation;
mod text;
pub use access::Access;
pub use presentation::{Panel, Presentation};
pub use text::Texts;

/// Logical frontend identities, never addresses reconstructed from an IFF.
/// All six panel containers must be present even before their contents work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owners {
    pub panel: u32,
    pub state: u32,
    pub controls: u32,
    pub lot: u32,
    pub manager: u32,
    pub panels: [u32; 6],
    pub widgets: [u32; 18],
    pub pause: u32,
}
impl Owners {
    fn claim(&self) -> PanelOwner {
        PanelOwner {
            panel: self.panel,
            controls: self.controls,
            state: self.state,
        }
    }
    fn identities(&self) -> impl Iterator<Item = u32> + '_ {
        [
            self.panel,
            self.state,
            self.controls,
            self.lot,
            self.manager,
            self.pause,
        ]
        .into_iter()
        .chain(self.panels)
        .chain(self.widgets)
    }
    fn validate(&self) -> Result<()> {
        let ids: Vec<_> = self.identities().collect();
        ensure!(
            !ids.contains(&0) && ids.iter().copied().collect::<BTreeSet<_>>().len() == ids.len(),
            "panel identities are null or aliased"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Button {
    selected: i32,
    auxiliary: i32,
    /// Set by the replacement Changed callback, after the native field write.
    painted_selected: bool,
    dirty: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonView {
    pub enabled: bool,
    pub selected: bool,
    pub auxiliary: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Setter {
    Selected(button::Setter),
    Auxiliary(button::AuxiliarySetter),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ButtonCall {
    widget: u32,
    value: i32,
    setter: Setter,
    pending: Option<button::Request>,
}
impl ButtonCall {
    fn matches(&self, request: buttons::Request) -> bool {
        match (request, &self.setter) {
            (buttons::Request::Selected { widget, value }, Setter::Selected(_))
            | (buttons::Request::Auxiliary { widget, value }, Setter::Auxiliary(_)) => {
                widget == self.widget && value == self.value
            }
            _ => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Selection {
    request: control::Request,
    call: panel_switch::Selection,
    visibility: Option<visibility::Visibility>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Execution {
    serial: u64,
    #[serde(default, skip_serializing_if = "Caller::publication")]
    caller: Caller,
    dispatch: dispatch::Update,
    control: Option<control::Update>,
    buttons: Option<(control::Request, buttons::Call)>,
    button: Option<ButtonCall>,
    selection: Option<Selection>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Caller {
    #[default]
    Publication,
    Property,
}
impl Caller {
    fn publication(&self) -> bool {
        *self == Self::Publication
    }
}
/// A precise unfinished child, useful to headless runners and the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Boundary {
    Dispatcher(dispatch::Request),
    Control(control::Request),
    Visibility(visibility::Call),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Widgets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contents: Option<catalog::Contents>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    presentation: Option<Presentation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    texts: Option<Texts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access: Option<Access>,
    owners: Owners,
    fields: control::Fields,
    buttons: buttons::Fields,
    enabled: BTreeMap<u32, u8>,
    widgets: BTreeMap<u32, Button>,
    dirty: BTreeSet<u32>,
    panels: panel_switch::Fields,
    /// Explicit frontend preference. Not inferred from household save data.
    visibility_policy: u8,
    call: Execution,
}
impl Widgets {
    pub(super) fn property_owner(&self) -> PanelOwner {
        self.owners.claim()
    }
    pub(super) fn validate_property_ids(&self, view: u32, renderer: u32) -> Result<()> {
        ensure!(
            !self
                .owners
                .identities()
                .any(|id| id == view || id == renderer),
            "property view/renderer aliases a panel widget"
        );
        Ok(())
    }
    pub(super) fn clear_product_selection(&mut self) -> Result<()> {
        self.contents
            .as_mut()
            .context("Buy catalog absent")?
            .buy
            .clear_product_selection();
        Ok(())
    }
    fn fresh(owners: Owners, visibility_policy: u8, serial: u64) -> Self {
        let ids: Vec<_> = owners.widgets.into_iter().chain([owners.pause]).collect();
        Self {
            contents: None,
            presentation: None,
            texts: None,
            access: None,
            fields: control::Fields {
                registered: owners.state,
                widgets: owners.widgets,
                // 537e79 initializes the interval/deadlines and both periods.
                // Phase is explicit replacement startup state, not a recovered
                // constructor write or an inferred original saved field.
                blink: control::Blink {
                    phase: 0,
                    interval: 0,
                    deadline: u32::MAX,
                    last: u32::MAX,
                    first: 250,
                    second: 200,
                },
            },
            buttons: buttons::Fields::constructed(),
            enabled: ids.iter().map(|&id| (id, 1)).collect(),
            widgets: ids
                .into_iter()
                .map(|id| {
                    (
                        id,
                        Button {
                            selected: 0,
                            auxiliary: 0,
                            painted_selected: false,
                            dirty: false,
                        },
                    )
                })
                .collect(),
            dirty: BTreeSet::new(),
            panels: panel_switch::Fields {
                selected: 0,
                panels: owners.panels,
            },
            call: Execution {
                serial,
                caller: Caller::Publication,
                dispatch: dispatch::Update::new(owners.state),
                control: None,
                buttons: None,
                button: None,
                selection: None,
            },
            visibility_policy,
            owners,
        }
    }
    pub fn button(&self, widget: control::Widget) -> ButtonView {
        self.button_id(self.fields.widget(widget))
    }
    pub fn presentation(&self) -> Option<&Presentation> {
        self.presentation.as_ref()
    }
    pub fn texts(&self) -> Option<&Texts> {
        self.texts.as_ref()
    }
    pub fn access(&self) -> Option<&Access> {
        self.access.as_ref()
    }
    pub fn catalog(&self) -> Option<&catalog::Catalog> {
        self.contents.as_ref().map(|c| &c.buy)
    }
    pub fn pause(&self) -> ButtonView {
        self.button_id(self.owners.pause)
    }
    fn button_id(&self, id: u32) -> ButtonView {
        let b = &self.widgets[&id];
        ButtonView {
            enabled: self.enabled[&id] != 0,
            selected: b.painted_selected,
            auxiliary: b.auxiliary,
        }
    }
    pub fn button_phase(&self) -> i32 {
        self.buttons.phase
    }
    pub fn boundary(&self) -> Option<Boundary> {
        if let Some(r) = self
            .call
            .selection
            .as_ref()
            .and_then(|s| s.visibility.as_ref())
            .and_then(visibility::Visibility::request)
        {
            return Some(Boundary::Visibility(r.clone()));
        }
        if let Some(r) = self
            .call
            .control
            .as_ref()
            .and_then(control::Update::request)
        {
            return Some(Boundary::Control(r));
        }
        self.call.dispatch.request().map(Boundary::Dispatcher)
    }
    pub fn button_callback_pending(&self) -> bool {
        self.call.button.is_some()
    }
    fn validate(&self) -> Result<()> {
        if let Some(contents) = &self.contents {
            ensure!(self.presentation.is_some(), "catalog presentation absent");
            contents.validate()?;
        }
        ensure!(
            (self.texts.is_none() && self.access.is_none()) || self.presentation.is_some(),
            "panel properties lost their presentation owner"
        );
        self.owners.validate()?;
        self.call.dispatch.validate().map_err(anyhow::Error::msg)?;
        let ids: BTreeSet<_> = self
            .owners
            .widgets
            .into_iter()
            .chain([self.owners.pause])
            .collect();
        ensure!(
            self.call.serial != 0
                && self.call.dispatch.matches(self.owners.state)
                && self.fields.registered == self.owners.state
                && self.fields.widgets == self.owners.widgets
                && self.panels.panels == self.owners.panels
                && self.enabled.keys().copied().collect::<BTreeSet<_>>() == ids
                && self.widgets.keys().copied().collect::<BTreeSet<_>>() == ids
                && self
                    .dirty
                    .iter()
                    .all(|id| self.owners.identities().any(|i| i == *id)),
            "panel widget ownership differs"
        );
        if let Some(c) = &self.call.control {
            c.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                c.matches(self.owners.controls, self.owners.state)
                    && self.call.dispatch.request()
                        == Some(dispatch::Request {
                            operation: dispatch::Operation::Controls,
                            child: Some(self.owners.controls),
                            state: Some(self.owners.state),
                        }),
                "ViewControl lost dispatcher"
            );
        } else {
            ensure!(
                self.call.buttons.is_none()
                    && self.call.button.is_none()
                    && self.call.selection.is_none(),
                "widget child lost ViewControl"
            );
        }
        if let Some((r, c)) = &self.call.buttons {
            c.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.call
                    .control
                    .as_ref()
                    .and_then(control::Update::request)
                    == Some(*r)
                    && matches!(r, control::Request::Buttons { view, selector }
                        if *view == self.owners.controls && c.matches(*view, buttons::Kind::Dispatch(*selector)))
                    && self.call.selection.is_none(),
                "button transition lost ViewControl"
            );
        }
        if let Some(c) = &self.call.button {
            ensure!(self.widgets.contains_key(&c.widget), "setter widget absent");
            let parent = self.call.buttons.as_ref().and_then(|(_, b)| b.request());
            let matches_direct = matches!(
                (self.call.control.as_ref().and_then(control::Update::request), &c.setter),
                (Some(control::Request::Selected {widget, value}), Setter::Selected(_))
                    if widget == c.widget && value == c.value && self.call.buttons.is_none()
            );
            ensure!(
                parent.is_some_and(|r| c.matches(r)) || matches_direct,
                "button setter lost its caller"
            );
            if let Setter::Selected(s) = &c.setter {
                s.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    s.matches(c.value) && s.request() == c.pending.as_ref(),
                    "button setter arguments/callback differ"
                );
            } else if let Setter::Auxiliary(s) = &c.setter {
                ensure!(
                    s.matches(c.value)
                        && c.pending
                            .as_ref()
                            .is_none_or(|r| *r == button::Request::Invalidate),
                    "auxiliary setter arguments/callback differ"
                );
            }
        }
        for (&id, b) in &self.widgets {
            let changing = self.call.button.as_ref().is_some_and(|c| {
                c.widget == id
                    && matches!(&c.setter, Setter::Selected(s)
                    if s.request() == Some(&button::Request::Changed))
            });
            ensure!(
                changing || b.painted_selected == (b.selected != 0),
                "button style lost Changed callback"
            );
        }
        if let Some(s) = &self.call.selection {
            s.call.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.call.buttons.is_none()
                    && self.call.button.is_none()
                    && self
                        .call
                        .control
                        .as_ref()
                        .and_then(control::Update::request)
                        == Some(s.request)
                    && matches!(s.request, control::Request::Panel { panel, selector }
                    if panel == self.owners.panel && s.call.matches(selector)),
                "panel switch lost ViewControl"
            );
            if let Some(v) = &s.visibility {
                v.validate().map_err(anyhow::Error::msg)?;
                let Some(panel_switch::Request::Visibility { panel, show }) = s.call.request()
                else {
                    anyhow::bail!("visibility lost panel selector");
                };
                ensure!(
                    self.owners.panels.contains(&panel) && v.matches(panel, show),
                    "visibility container/arguments differ"
                );
                if let Some(r) = v.request() {
                    ensure!(r.window == panel, "visibility target differs");
                }
            }
        }
        Ok(())
    }
    fn invalidate(&mut self, owner: u32) -> Result<()> {
        ensure!(
            owner == self.owners.controls
                || owner == self.owners.panel
                || self.owners.panels.contains(&owner)
                || self.widgets.contains_key(&owner),
            "invalidation owner absent"
        );
        if let Some(b) = self.widgets.get_mut(&owner) {
            b.dirty = true;
        }
        self.dirty.insert(owner);
        Ok(())
    }
    /// One setter operation, including a saved boundary before each callback.
    fn advance_button(&mut self) -> Result<()> {
        let c = self.call.button.as_mut().context("button caller absent")?;
        let b = self.widgets.get_mut(&c.widget).context("button absent")?;
        if let Some(r) = c.pending.clone() {
            let reply = match r {
                button::Request::Changed => {
                    b.painted_selected = b.selected != 0;
                    button::Reply::Done
                }
                button::Request::Invalidate => {
                    b.dirty = true;
                    self.dirty.insert(c.widget);
                    button::Reply::Done
                }
                button::Request::Parent => button::Reply::Parent(Some(self.owners.controls)),
                button::Request::InvalidateParent(parent) => {
                    ensure!(parent == self.owners.controls, "button parent changed");
                    self.dirty.insert(parent);
                    button::Reply::Done
                }
            };
            match &mut c.setter {
                Setter::Selected(s) => s.acknowledge(&r, reply).map_err(anyhow::Error::msg)?,
                Setter::Auxiliary(s) => s.acknowledge(&r).map_err(anyhow::Error::msg)?,
            }
            c.pending = None;
        } else {
            c.pending = match &mut c.setter {
                Setter::Selected(s) => s.advance(&mut b.selected).map_err(anyhow::Error::msg)?,
                Setter::Auxiliary(s) => s.advance(&mut b.auxiliary),
            };
            if c.pending.is_some() {
                return Ok(());
            }
            ensure!(
                match &c.setter {
                    Setter::Selected(s) => s.complete(),
                    Setter::Auxiliary(s) => s.complete(),
                },
                "button returned early"
            );
            if let Some((_, b)) = &mut self.call.buttons {
                let r = b.request().context("button transition request absent")?;
                ensure!(c.matches(r), "button reply arguments changed");
                b.acknowledge(r).map_err(anyhow::Error::msg)?;
            } else {
                let control = self.call.control.as_mut().context("ViewControl absent")?;
                let r = control.request().context("ViewControl request absent")?;
                control
                    .acknowledge(r, control::Reply::Done)
                    .map_err(anyhow::Error::msg)?;
            }
            self.call.button = None;
        }
        Ok(())
    }
    fn start_button(&mut self, r: buttons::Request) -> Result<()> {
        let (widget, value, setter) = match r {
            buttons::Request::Selected { widget, value } => {
                (widget, value, Setter::Selected(button::Setter::new(value)))
            }
            buttons::Request::Auxiliary { widget, value } => (
                widget,
                value,
                Setter::Auxiliary(button::AuxiliarySetter::new(value)),
            ),
            _ => anyhow::bail!("not a button setter"),
        };
        ensure!(
            self.widgets.contains_key(&widget),
            "button setter owner absent"
        );
        self.call.button = Some(ButtonCall {
            widget,
            value,
            setter,
            pending: None,
        });
        Ok(())
    }
    fn advance(&mut self, state: &mut State, window: &mut mode::Window) -> Result<bool> {
        self.validate()?;
        if self.call.button.is_some() {
            self.advance_button()?;
            return Ok(true);
        }
        if let Some((parent, call)) = &mut self.call.buttons {
            let (_, blocked, now) = window.panel_inputs();
            let input = buttons::Inputs {
                mode: state.mode,
                paused: state.paused,
                speed: state.speed,
                window_blocked: blocked,
                timer_owner: self.owners.manager,
                pause: self.owners.pause,
                ultra: self.fields.widget(control::Widget::Ultra),
            };
            if let Some(r) = call
                .advance(&mut self.buttons, input)
                .map_err(anyhow::Error::msg)?
            {
                match r {
                    buttons::Request::Register {
                        manager,
                        view,
                        interval,
                        argument,
                    } => {
                        ensure!(
                            manager == self.owners.manager && view == self.owners.controls,
                            "panel timer owner differs"
                        );
                        window
                            .timers_mut()
                            .register(Entry {
                                owner: view,
                                interval,
                                argument,
                                deadline: now.wrapping_add(interval),
                            })
                            .map_err(anyhow::Error::msg)?;
                        call.acknowledge(r).map_err(anyhow::Error::msg)?;
                    }
                    buttons::Request::Unregister { manager, view } => {
                        ensure!(
                            manager == self.owners.manager && view == self.owners.controls,
                            "panel timer owner differs"
                        );
                        window
                            .timers_mut()
                            .remove(view)
                            .map_err(anyhow::Error::msg)?;
                        call.acknowledge(r).map_err(anyhow::Error::msg)?;
                    }
                    _ => self.start_button(r)?,
                }
            } else {
                ensure!(call.complete(), "button transition returned early");
                self.call
                    .control
                    .as_mut()
                    .unwrap()
                    .acknowledge(*parent, control::Reply::Done)
                    .map_err(anyhow::Error::msg)?;
                self.call.buttons = None;
            }
            return Ok(true);
        }
        if let Some(s) = &mut self.call.selection {
            if let Some(v) = &mut s.visibility {
                if let Some(r) = v
                    .advance(self.visibility_policy)
                    .map_err(anyhow::Error::msg)?
                {
                    let Some(presentation) = &mut self.presentation else {
                        return Ok(false);
                    };
                    let index = self
                        .owners
                        .panels
                        .iter()
                        .position(|id| *id == r.window)
                        .context("visibility panel absent")?;
                    let panel = Panel::ALL[index];
                    let show = match (r.slot, r.arguments.as_slice()) {
                        (0x94, []) => true,
                        (0x98, []) => false,
                        (0xa8, [show, 0, 0]) => *show != 0,
                        _ => anyhow::bail!("unsupported panel visibility request"),
                    };
                    // Options Show also refreshes its settings/subpanels.
                    // Its content service has not been connected to Iced.
                    if panel == Panel::Options && show {
                        return Ok(false);
                    }
                    // Immediate presentation replaces cosmetic surface fades.
                    // The Iced view removes hidden children, and its input
                    // adapter rejects events from those children. Catalog and
                    // other panel content updates still have their own caller.
                    presentation.set_visible(panel, show);
                    self.dirty.insert(r.window);
                    v.acknowledge(&r).map_err(anyhow::Error::msg)?;
                }
                ensure!(v.complete(), "panel visibility returned early");
                let r = s.call.request().context("panel visibility caller absent")?;
                s.call.acknowledge(r).map_err(anyhow::Error::msg)?;
                s.visibility = None;
                return Ok(true);
            }
            match s
                .call
                .advance(&mut self.panels)
                .map_err(anyhow::Error::msg)?
            {
                Some(panel_switch::Request::Visibility { panel, show }) => {
                    s.visibility = Some(visibility::Visibility::new(panel, show));
                }
                Some(panel_switch::Request::Invalidate(id)) => {
                    let r = panel_switch::Request::Invalidate(id);
                    self.invalidate(id)?;
                    self.call
                        .selection
                        .as_mut()
                        .unwrap()
                        .call
                        .acknowledge(r)
                        .map_err(anyhow::Error::msg)?;
                }
                None => {
                    ensure!(s.call.complete(), "panel switch returned early");
                    self.call
                        .control
                        .as_mut()
                        .unwrap()
                        .acknowledge(s.request, control::Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                    self.call.selection = None;
                }
            }
            return Ok(true);
        }
        if let Some(c) = &mut self.call.control {
            let (_, _, now) = window.panel_inputs();
            let r = c
                .advance(
                    &mut self.fields,
                    state,
                    &mut self.enabled,
                    control::Inputs {
                        panel: self.owners.panel,
                        lot: self.owners.lot,
                        now,
                    },
                )
                .map_err(anyhow::Error::msg)?;
            match r {
                Some(control::Request::Invalidate(id)) => {
                    self.invalidate(id)?;
                    self.call
                        .control
                        .as_mut()
                        .unwrap()
                        .acknowledge(r.unwrap(), control::Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                Some(control::Request::Buttons { view, selector }) => {
                    ensure!(
                        view == self.owners.controls,
                        "ViewControl button owner differs"
                    );
                    self.call.buttons = Some((
                        r.unwrap(),
                        buttons::Call::new(view, buttons::Kind::Dispatch(selector)),
                    ));
                }
                Some(control::Request::Get {
                    owner,
                    query: control::Query::Mode,
                }) => {
                    ensure!(owner == self.owners.state, "CPState owner differs");
                    c.acknowledge(r.unwrap(), control::Reply::Number(state.mode as u32))
                        .map_err(anyhow::Error::msg)?;
                }
                Some(control::Request::Selected { widget, value }) => {
                    self.start_button(buttons::Request::Selected { widget, value })?
                }
                Some(control::Request::Panel { panel, selector }) => {
                    ensure!(panel == self.owners.panel, "panel switch owner differs");
                    self.call.selection = Some(Selection {
                        request: r.unwrap(),
                        call: panel_switch::Selection::new(selector),
                        visibility: None,
                    });
                }
                Some(control::Request::Expansion { view, value }) => {
                    ensure!(
                        view == self.owners.controls,
                        "panel expansion owner differs"
                    );
                    ensure!(matches!(value, 0 | 1), "panel expansion value unsupported");
                    let Some(presentation) = &mut self.presentation else {
                        return Ok(false);
                    };
                    if presentation.set_collapsed(value == 1) {
                        // Iced removes the panel's children and their input
                        // scope. Its sidebar does not cover the retained lot
                        // viewport, so no legacy surface resize is necessary.
                        self.dirty.insert(self.owners.panel);
                        self.dirty.insert(self.owners.controls);
                    }
                    c.acknowledge(r.unwrap(), control::Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                Some(r) => {
                    if let Some(access) = &mut self.access
                        && let Some(reply) = access.service(&self.owners, r)?
                    {
                        c.acknowledge(r, reply).map_err(anyhow::Error::msg)?;
                    } else if let Some(texts) = &mut self.texts
                        && let Some(reply) = texts.service(&self.owners, r)?
                    {
                        c.acknowledge(r, reply).map_err(anyhow::Error::msg)?;
                    } else {
                        return Ok(false);
                    }
                }
                None => {
                    ensure!(c.complete(), "ViewControl returned early");
                    let r = self
                        .call
                        .dispatch
                        .request()
                        .context("panel dispatcher absent")?;
                    self.call
                        .dispatch
                        .acknowledge(r)
                        .map_err(anyhow::Error::msg)?;
                    self.call.control = None;
                }
            }
            return Ok(true);
        }
        let p = &self.owners.panels;
        let r = self
            .call
            .dispatch
            .advance(dispatch::Inputs {
                mode: state.mode,
                dirty: state.buy.dirty,
                controls: Some(self.owners.controls),
                people: Some(p[0]),
                buy: Some(p[1]),
                build: Some(p[2]),
                options: Some(p[3]),
                camera: Some(p[5]),
            })
            .map_err(anyhow::Error::msg)?;
        match r {
            Some(dispatch::Request {
                operation: dispatch::Operation::Controls,
                ..
            }) => {
                self.call.control = Some(control::Update::new(
                    self.owners.controls,
                    self.owners.state,
                ));
                Ok(true)
            }
            Some(r)
                if self.contents.is_some()
                    && matches!(
                        r.operation,
                        dispatch::Operation::HideBuild
                            | dispatch::Operation::HideBuy
                            | dispatch::Operation::Buy
                    ) =>
            {
                ensure!(
                    r.child
                        == Some(
                            self.owners.panels[if r.operation == dispatch::Operation::HideBuild {
                                2
                            } else {
                                1
                            }]
                        ),
                    "catalog dispatcher child owner differs"
                );
                let contents = self.contents.as_mut().unwrap();
                match r.operation {
                    dispatch::Operation::HideBuild => contents.build_inspector = None,
                    dispatch::Operation::HideBuy => contents.buy.inspector = None,
                    dispatch::Operation::Buy => contents.buy.update(state)?,
                    _ => unreachable!(),
                }
                self.call
                    .dispatch
                    .acknowledge(r)
                    .map_err(anyhow::Error::msg)?;
                Ok(true)
            }
            // Other content owners remain explicit until their actual effects
            // are implemented. A completed dispatcher is returned by Device.
            _ => Ok(false),
        }
    }
}

pub(super) fn validate(s: &Session, saved: &Saved) -> Result<()> {
    let parent = s
        .host
        .presentation
        .publication
        .as_ref()
        .and_then(|p| p.panel_owner());
    let property = s
        .host
        .presentation
        .property_refresh
        .as_ref()
        .and_then(|p| p.panel_owner());
    match (&saved.panel_widgets, parent) {
        (None, None) => {
            ensure!(property.is_none(), "property caller lost its panel backend");
            Ok(())
        }
        (Some(w), owner) => {
            w.validate()?;
            if let Some(collapsed) = w.presentation.as_ref().and_then(Presentation::collapsed) {
                ensure!(
                    saved.panel_state == Some(i32::from(collapsed)),
                    "panel display differs from published presentation"
                );
            }
            ensure!(
                owner.is_none() || property.is_none(),
                "panel backend has concurrent callers"
            );
            let claimed = owner
                .map(|(serial, owner)| (Caller::Publication, serial, owner))
                .or_else(|| property.map(|(serial, owner)| (Caller::Property, serial, owner)));
            ensure!(
                claimed == Some((w.call.caller, w.call.serial, w.owners.claim()))
                    || (claimed.is_none() && w.call.dispatch.complete()),
                "panel backend lost its exact publication/property owner"
            );
            let window = saved.mode_window.as_ref().context("panel window absent")?;
            let (id, _, _) = window.panel_inputs();
            ensure!(
                !w.owners.identities().any(|i| i == id),
                "panel aliases mode window"
            );
            if w.call.buttons.is_none() {
                ensure!(
                    (w.buttons.timer_registered != 0)
                        == window
                            .timers()
                            .entries
                            .iter()
                            .any(|e| e.owner == w.owners.controls),
                    "panel timer flag differs from shared registry"
                );
            }
            Ok(())
        }
        _ => anyhow::bail!("panel frontend lost its saved caller"),
    }
}
impl Device {
    pub(super) fn property_panel_owner(&self) -> Option<PanelOwner> {
        self.saved.panel_widgets.as_ref().map(|w| w.owners.claim())
    }
    pub(super) fn property_text(
        &mut self,
        request: sims_vm::interface::property_refresh::Request,
        language: u8,
        bytes: Vec<u8>,
    ) -> Result<()> {
        self.saved
            .panel_widgets
            .as_mut()
            .context("property panel absent")?
            .texts
            .as_mut()
            .context("property text owner absent")?
            .property(request, language, bytes)
    }
    /// Complete the People child with our actual saved Iced panel owner. This
    /// is the replacement presentation contract, not execution of the legacy
    /// cWinLive window hierarchy, trend histories or undisplayed categories.
    /// Unknown dispatcher children remain suspended.
    pub fn publish_replacement_people(
        &mut self,
        s: &Session,
        client: &mut crate::dialog::client::Client,
    ) -> Result<bool> {
        validate(s, &self.saved)?;
        let Some(widgets) = self.saved.panel_widgets.as_ref() else {
            return Ok(false);
        };
        let Some(Boundary::Dispatcher(request)) = widgets.boundary() else {
            return Ok(false);
        };
        if !matches!(
            request.operation,
            dispatch::Operation::People | dispatch::Operation::InvalidatePeople
        ) {
            return Ok(false);
        }
        ensure!(
            request.child == Some(widgets.owners.panels[0]),
            "People publication owner differs"
        );
        ensure!(
            client.live_panel().is_some() && !client.pending(),
            "People widgets are absent or busy"
        );
        client.refresh_live_panel(s)?;
        let widgets = self.saved.panel_widgets.as_mut().unwrap();
        widgets.dirty.insert(widgets.owners.panels[0]);
        widgets
            .call
            .dispatch
            .acknowledge(request)
            .map_err(anyhow::Error::msg)?;
        validate(s, &self.saved)?;
        Ok(true)
    }
    /// Stale/hidden/disabled widget events are ignored just like Iced's tree.
    /// Accepted events are saved inside the Session's existing tool owner.
    pub fn begin_buy_product(
        &mut self,
        s: &mut Session,
        revision: u64,
        index: usize,
    ) -> Result<bool> {
        let Some(w) = &self.saved.panel_widgets else {
            return Ok(false);
        };
        if !w
            .presentation
            .as_ref()
            .is_some_and(|p| p.visible(Panel::Buy))
            || !w
                .catalog()
                .is_some_and(|c| c.accepts_product(revision, index))
        {
            return Ok(false);
        }
        let definition = w
            .catalog()
            .unwrap()
            .product(index)
            .context("Buy item absent")?
            .definition;
        s.begin_buy_product(catalog::Catalog::item_id(index)?, definition)?;
        self.saved
            .panel_widgets
            .as_mut()
            .unwrap()
            .contents
            .as_mut()
            .unwrap()
            .buy
            .insufficient_funds = false;
        Ok(true)
    }
    pub(super) fn service_buy_product(
        &mut self,
        s: &mut Session,
        request: sims_vm::interface::buy::product::Request,
    ) -> Result<()> {
        use sims_vm::interface::buy::product::{Reply, Request};
        let c = &mut self
            .saved
            .panel_widgets
            .as_mut()
            .and_then(|w| w.contents.as_mut())
            .context("Buy product frontend absent")?
            .buy;
        let reply = match request {
            Request::Read(index) => Reply::Slot(c.slot(index)?),
            Request::Cost(item) => {
                ensure!(item != 0, "Buy cost needs a real clicked item");
                let price = c
                    .product(item as usize - 1)
                    .context("Buy cost product absent")?
                    .fields
                    .price;
                let world = &s.simulation.vm.world;
                let funds = if *world.globals.get(&32).context("Buy lot kind absent")? == 1 {
                    999_999
                } else {
                    world
                        .budget
                        .as_ref()
                        .context("Buy household budget absent")?
                        .balance
                };
                Reply::Cost { funds, price }
            }
            Request::DetailsIfOpen(_) => {
                ensure!(
                    c.inspector.is_none(),
                    "open Buy product details needs its actual updater"
                );
                Reply::Done
            }
            Request::Widget { widget, selected } => {
                c.select_product_widget(widget, selected)?;
                Reply::Done
            }
            Request::ErrorFeedback => {
                s.buy_error_feedback_disabled_audio()?;
                c.insufficient_funds = true;
                Reply::Done
            }
            Request::BuyTool => anyhow::bail!("Buy tool bypassed real callback"),
        };
        s.acknowledge_buy_product(request, reply)
    }
    pub fn begin_buy_navigation(
        &self,
        s: &mut Session,
        revision: u64,
        event: sims_vm::interface::buy::input::Event,
    ) -> Result<bool> {
        let Some(w) = &self.saved.panel_widgets else {
            return Ok(false);
        };
        if !w
            .presentation
            .as_ref()
            .is_some_and(|p| p.visible(Panel::Buy))
            || !w.catalog().is_some_and(|c| c.accepts(revision, event))
        {
            return Ok(false);
        }
        s.begin_buy_navigation(event)?;
        Ok(true)
    }
    pub(super) fn service_buy_input(
        &mut self,
        s: &mut Session,
        request: sims_vm::interface::buy::input::Request,
    ) -> Result<()> {
        use sims_vm::interface::buy::input::Request;
        match request {
            Request::CategorySelected(selected) => self
                .saved
                .panel_widgets
                .as_mut()
                .and_then(|w| w.contents.as_mut())
                .context("catalog input frontend absent")?
                .buy
                .select_category(selected)?,
            Request::Tool { .. } => anyhow::bail!("indexed tool callback bypassed its real owner"),
        }
        s.acknowledge_buy_input(request)
    }
    /// Attach a fresh Iced catalog and closed content inspectors. This loads
    /// actual definitions/text but does not advance or acknowledge a caller.
    pub fn bind_panel_catalog(&mut self, s: &Session, page_capacity: usize) -> Result<()> {
        validate(s, &self.saved)?;
        let widgets = self
            .saved
            .panel_widgets
            .as_ref()
            .context("panel widgets absent")?;
        ensure!(
            widgets.presentation.is_some() && widgets.contents.is_none(),
            "catalog owner absent or already bound"
        );
        let contents =
            catalog::Contents::new(s.buy_products()?, s.buy_category_labels()?, page_capacity)?;
        self.saved.panel_widgets.as_mut().unwrap().contents = Some(contents);
        Ok(())
    }
    /// Construct replacement availability from the actual fire registry and
    /// lot-opening rules. Existing snapshots are not assigned constructor bytes.
    pub fn bind_panel_access(&mut self, s: &Session) -> Result<()> {
        validate(s, &self.saved)?;
        let sources = s.panel_access()?;
        let widgets = self
            .saved
            .panel_widgets
            .as_mut()
            .context("panel widgets absent")?;
        ensure!(widgets.presentation.is_some(), "panel presentation absent");
        ensure!(widgets.access.is_none(), "panel availability already bound");
        widgets.access = Some(Access::new(sources));
        Ok(())
    }
    /// The existing panel-toggle consumer and full ViewControl publication
    /// share the same actual replacement display property.
    pub(super) fn publish_panel_display(&mut self, value: i32) -> Result<()> {
        ensure!(matches!(value, 0 | 1), "panel display value unsupported");
        self.saved.panel_state = Some(value);
        if let Some(p) = self
            .saved
            .panel_widgets
            .as_mut()
            .and_then(|w| w.presentation.as_mut())
        {
            p.set_collapsed(value == 1);
        }
        Ok(())
    }
    /// Create the replacement funds/clock widgets from the current lot, using
    /// the native property reads. Historical saves remain unbound until the
    /// frontend explicitly installs these new widgets.
    pub fn bind_panel_text(&mut self, s: &Session) -> Result<()> {
        validate(s, &self.saved)?;
        let sources = s.panel_text()?;
        let widgets = self
            .saved
            .panel_widgets
            .as_mut()
            .context("panel widgets absent")?;
        ensure!(widgets.presentation.is_some(), "panel presentation absent");
        ensure!(widgets.texts.is_none(), "panel text already bound");
        widgets.texts = Some(Texts::new(sources));
        Ok(())
    }
    pub(super) fn refresh_panel_text(&mut self, s: &Session) -> Result<()> {
        if let Some(access) = self
            .saved
            .panel_widgets
            .as_mut()
            .and_then(|w| w.access.as_mut())
        {
            let mask = s
                .host
                .presentation
                .transport
                .as_ref()
                .context("panel state absent")?
                .controls
                .buy
                .dirty;
            if mask & 0x1000000 != 0 {
                access.refresh_fire(!s.simulation.vm.world.fires.registered.is_empty());
            }
        }
        if let Some(texts) = self
            .saved
            .panel_widgets
            .as_mut()
            .and_then(|w| w.texts.as_mut())
        {
            let mask = s
                .host
                .presentation
                .transport
                .as_ref()
                .context("panel state absent")?
                .controls
                .buy
                .dirty;
            texts.refresh(s.panel_text()?, mask)?;
        }
        Ok(())
    }
    /// Attach the actual replacement panel tree to the retained update. This
    /// does not acknowledge content updates, tutorial events, or mode changes.
    pub fn bind_panel_presentation(&mut self, s: &Session, initial: Panel) -> Result<()> {
        validate(s, &self.saved)?;
        let widgets = self
            .saved
            .panel_widgets
            .as_mut()
            .context("panel widgets absent")?;
        ensure!(
            widgets.presentation.is_none(),
            "panel presentation already bound"
        );
        widgets.presentation = Some(Presentation::new(initial));
        Ok(())
    }
    /// Explicitly construct new replacement widgets at the pending panel call.
    /// This does not reconstruct a suspended original Windows UI from a lot.
    pub fn bind_panel_widgets(
        &mut self,
        s: &mut Session,
        owners: Owners,
        visibility_policy: u8,
    ) -> Result<()> {
        owners.validate()?;
        ensure!(
            self.saved.panel_widgets.is_none(),
            "panel widgets already bound"
        );
        let window = self
            .saved
            .mode_window
            .as_ref()
            .context("panel window absent")?;
        let (id, _, _) = window.panel_inputs();
        ensure!(
            !owners.identities().any(|i| i == id)
                && !window
                    .timers()
                    .entries
                    .iter()
                    .any(|e| owners.identities().any(|i| i == e.owner)),
            "new panel identities collide with existing window/timers"
        );
        let serial = s.claim_publication_panels(owners.claim())?;
        self.saved.panel_widgets = Some(Widgets::fresh(owners, visibility_policy, serial));
        validate(s, &self.saved)
    }
    pub fn panel_widgets(&self) -> Option<&Widgets> {
        self.saved.panel_widgets.as_ref()
    }
    pub(super) fn advance_panel_widgets(&mut self, s: &mut Session, budget: usize) -> Result<bool> {
        validate(s, &self.saved)?;
        if self
            .saved
            .panel_widgets
            .as_ref()
            .is_some_and(|w| w.call.dispatch.complete())
        {
            let widgets = self.saved.panel_widgets.as_mut().unwrap();
            let property = s
                .host
                .presentation
                .property_refresh
                .as_ref()
                .and_then(|p| p.panel_request());
            let (caller, serial) = if property == Some(widgets.owners.claim()) {
                (
                    Caller::Property,
                    s.claim_property_panels(widgets.owners.claim())?,
                )
            } else {
                (
                    Caller::Publication,
                    s.claim_publication_panels(widgets.owners.claim())?,
                )
            };
            widgets.call = Execution {
                serial,
                caller,
                dispatch: dispatch::Update::new(widgets.owners.state),
                control: None,
                buttons: None,
                button: None,
                selection: None,
            };
        }
        for _ in 0..budget {
            let progressed = self
                .saved
                .panel_widgets
                .as_mut()
                .context("panel widgets absent")?
                .advance(
                    &mut s
                        .host
                        .presentation
                        .transport
                        .as_mut()
                        .context("panel CPState absent")?
                        .controls,
                    self.saved
                        .mode_window
                        .as_mut()
                        .context("panel window absent")?,
                )?;
            if let Some(collapsed) = self
                .saved
                .panel_widgets
                .as_ref()
                .and_then(|w| w.presentation.as_ref())
                .and_then(Presentation::collapsed)
            {
                self.saved.panel_state = Some(i32::from(collapsed));
            }
            validate(s, &self.saved)?;
            if !progressed {
                let widgets = self.saved.panel_widgets.as_mut().unwrap();
                if widgets.call.dispatch.complete() {
                    match widgets.call.caller {
                        Caller::Publication => s.complete_publication_panels(
                            widgets.call.serial,
                            widgets.owners.claim(),
                            &widgets.call.dispatch,
                        )?,
                        Caller::Property => s.complete_property_panels(
                            widgets.call.serial,
                            widgets.owners.claim(),
                            &widgets.call.dispatch,
                        )?,
                    }
                    validate(s, &self.saved)?;
                    return Ok(true);
                }
                if widgets.presentation.is_some()
                    && let Some(Boundary::Control(r @ control::Request::Get { owner, query })) =
                        widgets.boundary()
                {
                    let lot = matches!(
                        query,
                        control::Query::LotPaused
                            | control::Query::SavedSpeedActive
                            | control::Query::SavedSpeed
                            | control::Query::Speed
                    );
                    ensure!(
                        owner
                            == if lot {
                                widgets.owners.lot
                            } else {
                                widgets.owners.state
                            },
                        "panel getter owner differs"
                    );
                    if let Some(value) = s.panel_number(query)? {
                        widgets
                            .call
                            .control
                            .as_mut()
                            .unwrap()
                            .acknowledge(r, control::Reply::Number(value))
                            .map_err(anyhow::Error::msg)?;
                        continue;
                    }
                }
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn owners() -> Owners {
        Owners {
            panel: 10000,
            state: 10001,
            controls: 10002,
            lot: 10003,
            manager: 10004,
            panels: std::array::from_fn(|i| 10010 + i as u32),
            widgets: std::array::from_fn(|i| 10100 + i as u32),
            pause: 10200,
        }
    }
    #[test]
    fn collapse_and_expand_retain_panel_selection_and_resume_without_vm_or_timer_writes()
    -> Result<()> {
        fn run(restore: bool, policy: u8) -> Result<(Widgets, State, mode::Window)> {
            let mut w = Widgets::fresh(owners(), policy, 1);
            w.presentation = Some(Presentation::new(Panel::Buy));
            let mut state = State::constructed(760);
            state.mode = 1;
            state.buy.dirty = 0x100000;
            let before = state.clone();
            let mut window = mode::Window::new(9000, Some(9001), 80, 1, 1000);
            let timers = window.timers().clone();
            for (pass, value) in [1, 1, 0, 0].into_iter().enumerate() {
                w.call.dispatch = dispatch::Update::new(w.owners.state);
                w.dirty.clear();
                for n in 0..40 {
                    let progressed = w.advance(&mut state, &mut window)?;
                    w.validate()?;
                    if !progressed {
                        if let Some(Boundary::Control(
                            r @ control::Request::Get {
                                owner,
                                query: control::Query::Expansion,
                            },
                        )) = w.boundary()
                        {
                            ensure!(owner == w.owners.state);
                            w.call
                                .control
                                .as_mut()
                                .unwrap()
                                .acknowledge(r, control::Reply::Number(value))
                                .map_err(anyhow::Error::msg)?;
                        } else {
                            break;
                        }
                    }
                    if restore {
                        (w, state, window) =
                            serde_json::from_slice(&serde_json::to_vec(&(&w, &state, &window))?)?;
                    }
                    ensure!(n < 39, "expansion stalled");
                }
                ensure!(w.call.control.is_none());
                let p = w.presentation.as_ref().unwrap();
                ensure!(p.collapsed() == Some(value == 1));
                ensure!(p.root_visible() == (value == 0));
                ensure!(p.active() == if value == 0 { Some(Panel::Buy) } else { None });
                for panel in Panel::ALL {
                    ensure!(p.visible(panel) == (value == 0 && panel == Panel::Buy));
                }
                // Equal-state calls do not invalidate; reopening preserves the
                // selected child even after a cold serialization boundary.
                ensure!(w.dirty.is_empty() == matches!(pass, 1 | 3));
                ensure!(state == before && window.timers() == &timers);
                ensure!(matches!(w.boundary(), Some(Boundary::Dispatcher(_))));
            }
            Ok((w, state, window))
        }
        for policy in [0, 1, 128] {
            ensure!(run(false, policy)? == run(true, policy)?);
        }
        Ok(())
    }
    #[test]
    fn fire_and_lot_access_publish_enable_and_visibility_in_order_across_restore() -> Result<()> {
        fn run(blocked: bool, community: bool, restore: bool) -> Result<(Widgets, State)> {
            let mut w = Widgets::fresh(owners(), 0, 1);
            w.presentation = Some(Presentation::new(Panel::People));
            w.access = Some(Access::new(PanelAccess {
                buy_blocked: blocked,
                live_blocked: community,
                labels_hidden: community,
            }));
            let mut state = State::constructed(760);
            state.mode = 2;
            state.buy.dirty = 0x9000000;
            let before = state.clone();
            let mut window = mode::Window::new(9000, Some(9001), 80, 0, 1000);
            let before_window = window.clone();
            for n in 0..100 {
                if !w.access().unwrap().funds_visible() {
                    // Availability writes precede either label visibility.
                    ensure!(w.button(control::Widget::Buy).enabled == !blocked);
                    ensure!(w.button(control::Widget::Live).enabled == !community);
                }
                let progressed = w.advance(&mut state, &mut window)?;
                w.validate()?;
                if restore {
                    w = serde_json::from_slice(&serde_json::to_vec(&w)?)?;
                }
                if !progressed {
                    break;
                }
                ensure!(n < 99);
            }
            ensure!(w.call.control.is_none());
            ensure!(state == before && window == before_window);
            for widget in [control::Widget::Buy, control::Widget::Build] {
                ensure!(w.button(widget).enabled == !blocked);
            }
            for widget in [control::Widget::Live, control::Widget::Camera] {
                ensure!(w.button(widget).enabled == !community);
            }
            ensure!(w.access().unwrap().funds_visible() == !community);
            ensure!(w.access().unwrap().label_visible() == !community);
            ensure!(w.access().unwrap().secondary_text().is_empty());
            let saved = w.clone();
            ensure!(!w.advance(&mut state, &mut window)? && w == saved);
            Ok((w, state))
        }
        for blocked in [false, true] {
            for community in [false, true] {
                ensure!(run(blocked, community, false)? == run(blocked, community, true)?);
            }
        }
        Ok(())
    }
    #[test]
    fn funds_changes_keep_the_original_two_phase_publication_across_restore() -> Result<()> {
        use sims_runtime::original_session::publication::PanelText;
        fn run(restore: bool) -> Result<(Widgets, State)> {
            let mut w = Widgets::fresh(owners(), 0, 1);
            w.presentation = Some(Presentation::new(Panel::People));
            let mut state = State::constructed(760);
            state.mode = 2;
            let mut window = mode::Window::new(9000, Some(9001), 80, 1, 1000);
            w.texts = Some(Texts::new(PanelText {
                language: 1,
                funds: b"$1,000".to_vec(),
                clock: b"AM  6:33".to_vec(),
            }));
            for pass in 0..4 {
                state.buy.dirty = if pass == 0 { 0x6000 } else { 0x4000 };
                w.call.dispatch = dispatch::Update::new(w.owners.state);
                if pass == 1 {
                    w.texts.as_mut().unwrap().refresh(
                        PanelText {
                            language: 1,
                            funds: b"$750".to_vec(),
                            clock: b"ignored".to_vec(),
                        },
                        0x4000,
                    )?;
                }
                if pass >= 2 {
                    // The frame clock is explicit input; simulation time does
                    // not stand in for the original UI blink deadline.
                    window = mode::Window::new(9000, Some(9001), 80, 1, w.fields.blink.deadline);
                }
                for n in 0..200 {
                    let progressed = w.advance(&mut state, &mut window)?;
                    if restore {
                        (w, state, window) =
                            serde_json::from_slice(&serde_json::to_vec(&(&w, &state, &window))?)?;
                    }
                    if !progressed {
                        break;
                    }
                    ensure!(n < 199);
                }
                ensure!(w.call.control.is_none(), "text update did not finish");
                let t = w.texts.as_ref().unwrap();
                ensure!(t.clock() == "AM  6:33");
                ensure!(t.funds() == if pass <= 1 { "$1,000" } else { "$750" });
                ensure!(w.fields.blink.phase == [0, 1, 2, 0][pass]);
                ensure!(w.button(control::Widget::Status).selected == matches!(pass, 1 | 2));
                ensure!(
                    state.publication.as_ref().unwrap().after_dirty
                        == if matches!(pass, 1 | 2) { 0x4000 } else { 0 }
                );
            }
            Ok((w, state))
        }
        ensure!(run(false)? == run(true)?);
        Ok(())
    }
    fn run(restore: bool, policy: u8) -> Result<(Widgets, State, mode::Window)> {
        let mut w = Widgets::fresh(owners(), policy, 1);
        let mut state = State::constructed(760);
        state.mode = 1;
        state.buy.dirty = 0xfff7ffff;
        let before = state.clone();
        let mut window = mode::Window::new(9000, Some(9001), 80, 1, u32::MAX - 15);
        window
            .timers_mut()
            .register(Entry {
                owner: 8000,
                interval: 80,
                argument: 0,
                deadline: 64,
            })
            .map_err(anyhow::Error::msg)?;
        let timers = window.timers().clone();
        let mut saw_auxiliary_before_write = false;
        let mut saw_changed_before_style = false;
        let mut saw_parent = false;
        for n in 0..100 {
            let progressed = w.advance(&mut state, &mut window)?;
            w.validate()?;
            if let Some(c) = &w.call.button {
                if c.pending == Some(button::Request::Invalidate)
                    && matches!(c.setter, Setter::Auxiliary(_))
                {
                    ensure!(w.pause().auxiliary == 0);
                    saw_auxiliary_before_write = true;
                }
                if c.pending == Some(button::Request::Changed) {
                    ensure!(w.widgets[&w.owners.pause].selected == 1 && !w.pause().selected);
                    saw_changed_before_style = true;
                }
                if c.pending == Some(button::Request::Parent) {
                    ensure!(w.pause().selected);
                    saw_parent = true;
                }
            }
            if restore {
                (w, state, window) =
                    serde_json::from_slice(&serde_json::to_vec(&(&w, &state, &window))?)?;
                w.validate()?;
            }
            if !progressed {
                break;
            }
            ensure!(n < 99, "widget update stalled");
        }
        ensure!(saw_auxiliary_before_write && saw_changed_before_style && saw_parent);
        ensure!(state == before && window.timers() == &timers);
        ensure!(w.pause().selected && w.pause().auxiliary == 1 && w.buttons.phase == 1);
        // The selector is written before any child visibility. There is still
        // no successful panel return or mode-button update at this boundary.
        ensure!(w.panels.selected == 1 && !w.button(control::Widget::Buy).selected);
        ensure!(
            w.boundary()
                == Some(Boundary::Visibility(visibility::Call::new(
                    w.owners.panels[0],
                    if policy == 0 { 0x98 } else { 0xa8 },
                    if policy == 0 { &[] } else { &[0, 0, 0] },
                )))
        );
        let saved = w.clone();
        for _ in 0..3 {
            ensure!(!w.advance(&mut state, &mut window)? && w == saved);
        }
        Ok((w, state, window))
    }

    #[test]
    fn buy_widgets_resume_each_callback_and_keep_unfinished_visibility_pending() -> Result<()> {
        for policy in [0, 1, 128] {
            ensure!(run(false, policy)? == run(true, policy)?);
        }
        Ok(())
    }
    #[test]
    fn saved_widget_call_rejects_changed_owner_arguments_and_detached_children() -> Result<()> {
        let (w, _, _) = run(false, 1)?;
        let saved = serde_json::to_value(w)?;
        for (pointer, value) in [
            ("/call/control", Value::Null),
            ("/call/control/view", json!(777)),
            ("/call/selection/request/Panel/panel", json!(777)),
            ("/call/selection/call/selector", json!(0)),
            ("/call/selection/visibility/window", json!(777)),
            ("/call/selection/visibility/show", json!(1)),
            ("/fields/registered", json!(777)),
            ("/widgets/10200/painted_selected", json!(false)),
            ("/enabled/10200", Value::Null),
        ] {
            let mut corrupt = saved.clone();
            *corrupt
                .pointer_mut(pointer)
                .with_context(|| pointer.to_owned())? = value;
            ensure!(
                !serde_json::from_value::<Widgets>(corrupt).is_ok_and(|w| w.validate().is_ok()),
                "accepted {pointer}"
            );
        }
        Ok(())
    }
    #[test]
    fn replacement_panel_visibility_and_mode_selection_agree_across_policies_and_restore()
    -> Result<()> {
        for mode in [0, 1, 2, 3, 4] {
            let expected_panel = [
                Panel::Build,
                Panel::Buy,
                Panel::People,
                Panel::Options,
                Panel::Camera,
            ][mode as usize];
            let mut results = Vec::new();
            for policy in [0, 1, 128] {
                for restore in [false, true] {
                    let mut w = Widgets::fresh(owners(), policy, 1);
                    w.presentation = Some(Presentation::new(Panel::People));
                    let mut state = State::constructed(760);
                    state.mode = mode;
                    state.buy.dirty = 2;
                    let before = state.clone();
                    let mut window = mode::Window::new(9000, Some(9001), 80, 0, 0);
                    let mut stopped = false;
                    for _ in 0..300 {
                        if !w.advance(&mut state, &mut window)? {
                            stopped = true;
                            break;
                        }
                        if restore {
                            w = serde_json::from_slice(&serde_json::to_vec(&w)?)?;
                        }
                    }
                    ensure!(stopped && state == before);
                    if mode == 3 {
                        // Do not collapse Options' content-refreshing Show into
                        // a base visibility acknowledgement.
                        ensure!(matches!(w.boundary(), Some(Boundary::Visibility(_))));
                        ensure!(!w.presentation().unwrap().visible(Panel::Options));
                    } else {
                        ensure!(
                            w.boundary()
                                == Some(Boundary::Control(control::Request::Get {
                                    owner: w.owners.lot,
                                    query: control::Query::SavedSpeedActive,
                                }))
                        );
                        ensure!(w.presentation().unwrap().active() == Some(expected_panel));
                        for panel in Panel::ALL {
                            ensure!(
                                w.presentation().unwrap().visible(panel)
                                    == (panel == expected_panel)
                            );
                        }
                        for (widget, selected_mode) in [
                            (control::Widget::Live, 2),
                            (control::Widget::Buy, 1),
                            (control::Widget::Build, 0),
                            (control::Widget::Options, 3),
                            (control::Widget::Camera, 4),
                        ] {
                            ensure!(w.button(widget).selected == (mode == selected_mode));
                        }
                    }
                    let before = w.clone();
                    ensure!(!w.advance(&mut state, &mut window)? && w == before);
                    results.push((w.presentation.clone(), w.widgets.clone()));
                }
            }
            ensure!(results.windows(2).all(|pair| pair[0] == pair[1]));
        }
        Ok(())
    }
}
