//! Registered-window pass over frozen B read-footprint fields. Live action
//! headers are borrowed from their existing owner. Region queues own their
//! actual backing; native coalescing/growth remain explicit pending children.
use super::{Memory, Result, ensure, word};
use serde::{Deserialize, Serialize};
use sims_vm::{
    scheduler::clock::draw::{framework as native, regions},
    ui_effect::widgets::surface_geometry,
};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Widget {
    table: u32,
    parent: u32,
    flags: u32,
    rectangle: [i32; 4],
    suppress: u8,
}
impl Widget {
    fn read(h: &[u8]) -> Self {
        Self {
            table: word(h, 0),
            parent: word(h, 0x40),
            flags: word(h, 0x9c),
            rectangle: std::array::from_fn(|i| word(h, 12 + i * 4) as i32),
            suppress: h[0x6c],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Notification {
    window: u32,
    force: u32,
    checked: bool,
    pass: u8,
    frame: Option<regions::Frame>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    fields: native::Fields,
    frame: native::Frame,
    widgets: BTreeMap<u32, Widget>,
    tables: BTreeMap<u32, Vec<u8>>,
    held: bool,
    clips: [[i32; 4]; 2],
    enabled: u32,
    lists: BTreeMap<u16, regions::List>,
    notification: Option<Notification>,
    notifications: Vec<(u32, [i32; 4])>,
}
pub(super) fn import(
    source: &[u8],
    shared: &sims_vm::scheduler::clock::draw::actions::State,
) -> Result<Owner> {
    ensure!(
        sims_fixtures::digest(source)
            == "353fc5f47395e2c4ebc9a77f5841dc9c3770c6278776961766af1bc7642a1c10",
        "framework source differs"
    );
    let value: serde_json::Value = serde_json::from_slice(source)?;
    let m = Memory::new(&value)?;
    let h = m.read(0x6fedf8, 0x240)?;
    let (begin, end) = (word(&h, 0x34), word(&h, 0x38));
    let raw = m.read(begin, (end - begin) as usize)?;
    let registered: Vec<u32> = raw.chunks_exact(4).map(|p| word(p, 0)).collect();
    let mut widgets = BTreeMap::new();
    let mut tables = BTreeMap::new();
    for &p in &registered {
        let raw = m.read(p, 0xd4)?;
        let w = Widget::read(&raw);
        tables.entry(w.table).or_insert(m.read(w.table, 0xd8)?);
        if let Some(h) = shared.headers.get(&p) {
            ensure!(
                w == Widget::read(h),
                "framework action header alias differs"
            );
        } else {
            widgets.insert(p, w);
        }
    }
    let mut parents: Vec<u32> = widgets.values().map(|w| w.parent).collect();
    while let Some(parent) = parents.pop() {
        if parent == 0 || widgets.contains_key(&parent) || shared.headers.contains_key(&parent) {
            continue;
        }
        let w = Widget::read(&m.read(parent, 0xd4)?);
        tables.entry(w.table).or_insert(m.read(w.table, 0xd8)?);
        parents.push(w.parent);
        widgets.insert(parent, w);
    }
    let factory = word(&h, 0x220);
    ensure!(
        factory == 0x030f5f28,
        "framework factory cache not warm/source differs"
    );
    let region = m.read(factory + 0xb4, 0x98)?;
    let clip = word(&region, 0x78);
    let raw = m.read(clip + 0x2a4, 32)?;
    let clips = std::array::from_fn(|j| std::array::from_fn(|i| word(&raw, j * 16 + i * 4) as i32));
    let holder = m.read(word(&region, 0x94), 12)?;
    let manager = word(&holder, 8);
    ensure!(manager == 0x0313e6a0, "framework rectangle manager differs");
    let mut lists = BTreeMap::new();
    for offset in [0x450, 0x468, 0x480] {
        let header = m.read(manager + offset, 24)?;
        let backing = m.read(
            word(&header, 0),
            (word(&header, 8) - word(&header, 0)) as usize,
        )?;
        lists.insert(offset as u16, regions::List { header, backing });
    }
    let fields = native::Fields {
        owner: 0x6fedf8,
        root: word(&h, 0x2c),
        registered,
    };
    let result = Owner {
        frame: native::Frame::new(fields.owner),
        fields,
        widgets,
        tables,
        held: false,
        clips,
        enabled: word(&m.read(0x6d61b4, 4)?, 0),
        lists,
        notification: None,
        notifications: vec![],
    };
    result.validate(shared)?;
    Ok(result)
}
impl Owner {
    fn immutable_digest(&self) -> String {
        let mut b = Vec::new();
        for (&id, w) in &self.widgets {
            for x in [id, w.table, w.parent, w.flags]
                .into_iter()
                .chain(w.rectangle.map(|x| x as u32))
            {
                b.extend(x.to_le_bytes());
            }
            b.push(w.suppress);
        }
        sims_fixtures::digest(&b)
    }
    fn window(
        &self,
        shared: &sims_vm::scheduler::clock::draw::actions::State,
        p: u32,
    ) -> Result<Widget> {
        if let Some(h) = shared.headers.get(&p) {
            Ok(Widget::read(h))
        } else {
            self.widgets
                .get(&p)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("framework window {p:x} absent"))
        }
    }
    fn target(
        &self,
        shared: &sims_vm::scheduler::clock::draw::actions::State,
        p: u32,
        slot: u16,
        expected: u32,
    ) -> Result<()> {
        let w = self.window(shared, p)?;
        let table = self
            .tables
            .get(&w.table)
            .ok_or_else(|| anyhow::anyhow!("framework class absent"))?;
        ensure!(
            word(table, slot as usize) == expected,
            "framework window class requires another callback"
        );
        Ok(())
    }
    pub(super) fn at_entry(&self) -> bool {
        self.frame.at_entry()
    }
    pub(super) fn complete(&self) -> bool {
        self.frame.complete()
    }
    pub(super) fn pending_region(&self) -> Option<&regions::Request> {
        self.notification
            .as_ref()
            .and_then(|n| n.frame.as_ref())
            .and_then(regions::Frame::pending)
    }
    pub(super) fn validate(
        &self,
        shared: &sims_vm::scheduler::clock::draw::actions::State,
    ) -> Result<()> {
        self.frame
            .validate(&self.fields)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.fields.owner == 0x6fedf8
                && self.fields.root == 0x030f6088
                && self.fields.registered.len() == 2725
                && self.widgets.len() == 2724
                && self.immutable_digest()
                    == "a137d6f1e3a79fa2517ac02b3424b3385da06b3fb5aab23a40e22fa7542efe4c",
            "framework read-footprint owners differ"
        );
        let roster: Vec<u8> = self
            .fields
            .registered
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        ensure!(
            sims_fixtures::digest(&roster)
                == "0323df6edf7653f66cccd51259a7f4528498fa2924a2483c3252c8895edb2fa7",
            "framework roster differs"
        );
        let expected_held = !self.frame.at_entry() && !self.frame.complete();
        ensure!(
            self.held == expected_held,
            "framework critical ownership differs"
        );
        ensure!(
            self.enabled == 1 && self.clips == [[0, 0, 800, 600], [0, 0, 0, 0]],
            "framework region clip/enable source differs"
        );
        for (offset, begin) in [
            (0x450, 0x031416d0),
            (0x468, 0x031456d8),
            (0x480, 0x031496e0),
        ] {
            let list = self
                .lists
                .get(&offset)
                .ok_or_else(|| anyhow::anyhow!("framework rectangle queue absent"))?;
            list.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                word(&list.header, 0) == begin && word(&list.header, 8) == begin + 0x4000,
                "framework rectangle allocation changed without growth"
            );
        }
        ensure!(self.lists.len() == 3, "framework rectangle queues differ");
        if let Some(n) = &self.notification {
            ensure!(
                self.frame.pending()
                    == Some(&native::Request::Notify {
                        window: n.window,
                        force: n.force
                    })
                    && n.pass < 2,
                "framework region child escaped caller"
            );
            self.window(shared, n.window)?;
            if let Some(frame) = &n.frame {
                frame
                    .validate(&self.lists[&if n.pass == 0 { 0x480 } else { 0x450 }])
                    .map_err(anyhow::Error::msg)?;
            }
        }
        Ok(())
    }
    fn advance_notification(
        &mut self,
        shared: &sims_vm::scheduler::clock::draw::actions::State,
    ) -> Result<(bool, bool)> {
        let n = self.notification.as_ref().unwrap().clone();
        let w = self.window(shared, n.window)?;
        if !n.checked {
            self.notification.as_mut().unwrap().checked = true;
            if w.suppress != 0 && n.force & 255 == 0 {
                return Ok((true, true));
            }
        }
        let offset = if n.pass == 0 { 0x480 } else { 0x450 };
        if n.frame.is_none() {
            let (rect, result) =
                surface_geometry::intersection(w.rectangle, self.clips[n.pass as usize]);
            if result != 0 && rect[2] > rect[0] && rect[3] > rect[1] {
                self.notification.as_mut().unwrap().frame = Some(regions::Frame::new(rect));
            } else {
                if n.pass == 1 {
                    return Ok((true, true));
                }
                self.notification.as_mut().unwrap().pass = 1;
                return Ok((true, false));
            }
        }
        let frame = self.notification.as_mut().unwrap().frame.as_mut().unwrap();
        let list = self.lists.get_mut(&offset).unwrap();
        if let Some(request) = frame.pending().cloned() {
            match request {
                regions::Request::Append { rectangle } => {
                    if !list.append(rectangle).map_err(anyhow::Error::msg)? {
                        return Ok((false, false));
                    }
                    frame
                        .acknowledge(list, &request)
                        .map_err(anyhow::Error::msg)?;
                }
                regions::Request::Coalesce => return Ok((false, false)),
            }
        }
        if frame
            .advance(list, self.enabled)
            .map_err(anyhow::Error::msg)?
            .is_some()
        {
            return Ok((true, false));
        }
        if n.pass == 1 {
            return Ok((true, true));
        }
        let n = self.notification.as_mut().unwrap();
        n.frame = None;
        n.pass = 1;
        Ok((true, false))
    }
    pub(super) fn advance(
        &mut self,
        shared: &sims_vm::scheduler::clock::draw::actions::State,
    ) -> Result<bool> {
        self.validate(shared)?;
        struct Queries<'a> {
            owner: &'a Owner,
            shared: &'a sims_vm::scheduler::clock::draw::actions::State,
        }
        impl native::Query for Queries<'_> {
            fn flags(&mut self, p: u32, mask: u32) -> std::result::Result<u32, String> {
                self.owner
                    .target(self.shared, p, 0x8c, 0x436fef)
                    .map_err(|e| e.to_string())?;
                Ok(u32::from(
                    self.owner
                        .window(self.shared, p)
                        .map_err(|e| e.to_string())?
                        .flags
                        & mask
                        != 0,
                ))
            }
            fn parent(&mut self, p: u32) -> std::result::Result<u32, String> {
                self.owner
                    .target(self.shared, p, 0x18, 0x436e78)
                    .map_err(|e| e.to_string())?;
                Ok(self
                    .owner
                    .window(self.shared, p)
                    .map_err(|e| e.to_string())?
                    .parent)
            }
        }
        use native::Query;
        for _ in 0..20000 {
            let Some(request) = self
                .frame
                .advance(&self.fields)
                .map_err(anyhow::Error::msg)?
            else {
                self.validate(shared)?;
                return Ok(true);
            };
            let reply = match request {
                native::Request::Enter { critical } => {
                    ensure!(
                        critical == 0x6feff8 && !self.held,
                        "framework lock already held"
                    );
                    self.held = true;
                    native::Reply::Done
                }
                native::Request::Leave { critical } => {
                    ensure!(critical == 0x6feff8 && self.held, "framework lock not held");
                    self.held = false;
                    native::Reply::Done
                }
                native::Request::Flags { window, mask } => native::Reply::Value(
                    Queries {
                        owner: self,
                        shared,
                    }
                    .flags(window, mask)
                    .map_err(anyhow::Error::msg)?,
                ),
                native::Request::Parent { window } => native::Reply::Value(
                    Queries {
                        owner: self,
                        shared,
                    }
                    .parent(window)
                    .map_err(anyhow::Error::msg)?,
                ),
                native::Request::Visible { window } => native::Reply::Value(
                    native::visible(
                        window,
                        &mut Queries {
                            owner: self,
                            shared,
                        },
                    )
                    .map_err(anyhow::Error::msg)?,
                ),
                native::Request::Descendant { window, ancestor } => {
                    self.target(shared, window, 0x3c, 0x5dc8bd)?;
                    native::Reply::Value(
                        native::descendant(
                            window,
                            ancestor,
                            &mut Queries {
                                owner: self,
                                shared,
                            },
                        )
                        .map_err(anyhow::Error::msg)?,
                    )
                }
                native::Request::Notify { window, force } => {
                    self.target(shared, window, 0xd4, 0x5de1e1)?;
                    if self.notification.is_none() {
                        self.notifications
                            .push((window, self.window(shared, window)?.rectangle));
                        self.notification = Some(Notification {
                            window,
                            force,
                            checked: false,
                            pass: 0,
                            frame: None,
                        });
                    }
                    let (progress, done) = self.advance_notification(shared)?;
                    if !done {
                        self.validate(shared)?;
                        return Ok(progress);
                    }
                    self.notification = None;
                    native::Reply::Done
                }
            };
            self.frame
                .acknowledge(&self.fields, &request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        anyhow::bail!("framework read-only query budget exceeded")
    }
}
#[cfg(test)]
impl Owner {
    pub(super) fn compare_native(&self, raw: &[u8]) -> Result<()> {
        ensure!(
            sims_fixtures::digest(raw)
                == "5dd2b3582b573137f2633b4e74c198ef06cfe38d3a938cf75d867d9df897c9ad",
            "original framework result differs"
        );
        let result: serde_json::Value = serde_json::from_slice(raw)?;
        let decode = |v: &serde_json::Value| -> Result<Vec<u8>> {
            let s = v
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("native region bytes absent"))?;
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
                .collect()
        };
        let events: Vec<serde_json::Value> = self
            .notifications
            .iter()
            .map(|(window, rectangle)| serde_json::json!({"window":window,"rectangle":rectangle}))
            .collect();
        ensure!(
            serde_json::json!(events) == result["notifications"],
            "original framework notification order differs"
        );
        for row in result["rectangles_after"].as_array().unwrap() {
            let list = &self.lists[&(row["offset"].as_u64().unwrap() as u16)];
            ensure!(
                list.header == decode(&row["header"])?
                    && list.backing == decode(&row["allocation"])?,
                "original rectangle queue/header/backing differs"
            );
        }
        ensure!(
            self.complete() && !self.held,
            "framework did not return/release critical ownership"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private frozen framework roster and complete original region result"]
    fn frozen_framework_regions_match_original_and_binary_cuts() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let load = |p: &str| std::fs::read(root.join(p)).map_err(Into::<anyhow::Error>::into);
        let actions = super::super::action_view::import(&load(
            "fixtures/private/goth-action-refresh-20261031-first/action-source-first.json",
        )?)?;
        let mut state = import(
            &load("fixtures/private/goth-framework-20261031-first/framework-source-fourth.json")?,
            actions.state(),
        )?;
        let initial_actions = postcard::to_allocvec(&actions)?;
        let mut cuts = 0;
        while !state.complete() {
            let saved = postcard::to_allocvec(&state)?;
            let mut cold: Owner = postcard::from_bytes(&saved)?;
            cold.validate(actions.state())?;
            assert!(state.advance(actions.state())?);
            assert!(cold.advance(actions.state())?);
            assert_eq!(
                postcard::to_allocvec(&state)?,
                postcard::to_allocvec(&cold)?
            );
            cuts += 1;
            ensure!(cuts < 32, "framework did not return");
        }
        state.compare_native(&load(
            "fixtures/private/goth-framework-20261031-first/native-goth-third.json",
        )?)?;
        assert_eq!(postcard::to_allocvec(&actions)?, initial_actions);
        assert!(cuts >= 4);
        Ok(())
    }
}

impl Owner {
    /// The World renderer consumes the same backend queue populated by the
    /// preceding widget notifications. Do not re-import its frozen B contents.
    pub(super) fn drawing_regions(&self)->Result<&regions::List>{
        ensure!(self.complete()&&!self.held&&self.notification.is_none(),"World regions precede framework completion");
        self.lists.get(&0x480).ok_or_else(||anyhow::anyhow!("framework drawing queue absent"))
    }
    pub(super) fn drawing_regions_mut(&mut self)->Result<&mut regions::List>{
        self.drawing_regions()?;self.lists.get_mut(&0x480).ok_or_else(||anyhow::anyhow!("framework drawing queue absent"))
    }
}
