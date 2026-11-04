//!48ba19 constructs ObjectUndoable and recursively owns commands for occupied
//!physical slots across its multipart chain. Allocation identities are owned
//!by the Rust caller; None preserves the original nullable allocation result.
use super::{Class, Object, Objects};
use crate::edit_history::{
    List, Store, Vector,
    snapshot::{self, CaptureView, References, Snapshot},
};
use std::collections::BTreeSet;

pub trait View: CaptureView {
    ///Retained object+11c definition+60 OBJD word at byte8a.
    fn special(&self, object: u32) -> Result<bool, String>;
    ///Number of physical slot records excluding the sentinel at index0.
    fn slots(&self, object: u32) -> Result<usize, String>;
    ///Fresh current bounds and signed slot ID lookup for physical index n+1.
    fn child(&self, object: u32, n: usize) -> Result<Option<u32>, String>;
    ///Only nonzero multipart groups attempt the cXMTObject cast/next pointer.
    fn next_part(&self, object: u32) -> Result<Option<u32>, String>;
}
#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub command: u32,
    pub script_id: i16,
    pub kind: i32,
    pub cost: i32,
    pub budget: u32,
}
pub fn construct(
    input: Input,
    store: &mut Store,
    objects: &mut Objects,
    refs: &mut References,
    view: &mut impl View,
    allocate: &mut impl FnMut() -> Result<Option<u32>, String>,
) -> Result<(), String> {
    fn inner(
        i: Input,
        s: &mut Store,
        o: &mut Objects,
        r: &mut References,
        v: &mut impl View,
        a: &mut impl FnMut() -> Result<Option<u32>, String>,
        path: &mut BTreeSet<u32>,
    ) -> Result<(), String> {
        if o.0.contains_key(&i.command) || s.lists.contains_key(&List::Objects(i.command)) {
            return Err("object constructor receiver already exists".into());
        }
        s.construct_command(i.command)?;
        s.lists.insert(List::Objects(i.command), Vector::default());
        o.0.insert(
            i.command,
            Object {
                cost: None,
                budget: None,
                kind: i.kind,
                special: None,
                snapshots: [Snapshot::default(), Snapshot::default()],
                class: Class::Object,
            },
        );
        //Capture's AL is ignored by the constructor. Prefix state is retained
        //if a Rust diagnostic interrupts one of its mandatory accesses.
        snapshot::capture(&mut o.get_mut(i.command)?.snapshots[0], r, i.script_id, v)?;
        let object = o.get_mut(i.command)?;
        object.cost = Some(i.cost);
        object.budget = Some(i.budget);
        let target = r.object(&object.snapshots[0])?;
        object.special = Some(0);
        let Some(first) = target else {
            return Ok(());
        };
        object.special = Some(u8::from(v.special(first)?));
        if !path.insert(first) {
            return Err("cyclic object-command containment".into());
        }
        let mut parts = BTreeSet::new();
        let mut current = Some(first);
        while let Some(part) = current {
            if !parts.insert(part) {
                return Err("cyclic object-command multipart chain".into());
            }
            let count = v.slots(part)?;
            for n in 0..count {
                if let Some(child) = v.child(part, n)? {
                    let allocation = a()?;
                    if let Some(command) = allocation {
                        inner(
                            Input {
                                command,
                                script_id: v.script_id(child)?,
                                kind: i.kind,
                                cost: 0,
                                budget: i.budget,
                            },
                            s,
                            o,
                            r,
                            v,
                            a,
                            path,
                        )?;
                    }
                    s.append(List::Objects(i.command), allocation.unwrap_or(0))?;
                }
            }
            current = v.next_part(part)?;
        }
        path.remove(&first);
        Ok(())
    }
    inner(
        input,
        store,
        objects,
        refs,
        view,
        allocate,
        &mut BTreeSet::new(),
    )
}

impl View for snapshot::live::View<'_> {
    fn special(&self, object: u32) -> Result<bool, String> {
        let id = CaptureView::script_id(self, object)?;
        let definition = self
            .world
            .objects
            .get(&id)
            .and_then(|o| o.definition.as_ref())
            .ok_or("object command retained definition absent")?;
        self.program
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .and_then(|w| w.get(0x8a / 2))
            .map(|&v| v != 0)
            .ok_or("object command special-category field absent".into())
    }
    fn slots(&self, object: u32) -> Result<usize, String> {
        Ok(self
            .world
            .objects
            .get(&CaptureView::script_id(self, object)?)
            .ok_or("object command receiver absent")?
            .slots
            .len())
    }
    fn child(&self, object: u32, n: usize) -> Result<Option<u32>, String> {
        let id = CaptureView::script_id(self, object)?;
        let child = self
            .world
            .objects
            .get(&id)
            .ok_or("object command slot receiver absent")?
            .slots
            .get(n)
            .copied()
            .flatten();
        match child.filter(|&v| v > 0) {
            Some(id) => snapshot::Registry::lookup(self, id),
            None => Ok(None),
        }
    }
    fn next_part(&self, object: u32) -> Result<Option<u32>, String> {
        let id = CaptureView::script_id(self, object)?;
        let o = self
            .world
            .objects
            .get(&id)
            .ok_or("object command part absent")?;
        let d = o
            .definition
            .as_ref()
            .ok_or("object command part definition absent")?;
        let group = *self
            .program
            .modules
            .get(&d.source)
            .and_then(|m| m.object_definition_words.get(&d.object))
            .and_then(|w| w.get(10))
            .ok_or("object command part group absent")?;
        if group == 0 {
            return Ok(None);
        }
        match o.multitile.as_ref().and_then(|p| p.next) {
            None => Ok(None),
            Some(id) => Ok(Some(
                snapshot::Registry::lookup(self, id)?
                    .ok_or("object command next-part lifetime ended")?,
            )),
        }
    }
}
