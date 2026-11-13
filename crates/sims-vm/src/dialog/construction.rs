//! Suspension inside 43601c, before 4443ab publishes the constructed record.
//! No ordinary lot work or second execution of the Dialog opcode is permitted
//! while the asset host runs the portrait's reentrant callbacks.
use super::{Frame, Program, Record, Status, Vm, World};
use serde::{Deserialize, Serialize};

pub trait NeighborHost {
    fn neighbor(&mut self, id: i16) -> Result<Option<u32>, String>;
    fn name(&mut self, resource: u32) -> Result<String, String>;
    /// Resolve the exact definition held by a live object. A latest-by-GUID
    /// lookup can select a different resource after an override was registered.
    fn live(&mut self, _definition: &crate::Definition) -> Result<Option<u32>, String> {
        Err("dialog live resource host unavailable".into())
    }
    fn guid(&mut self, _guid: u32) -> Result<Option<u32>, String> {
        Err("dialog GUID resource host unavailable".into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    pub record: Record,
    /// Borrowed catalog definition handle, not a live object ID.
    pub resource: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relationship: bool,
}

/// Original text passes followed by 436279..43634d resource selection. This
/// read-only phase captures a borrowed catalog identity before image callbacks.
/// It can be compared independently from the VM's construction continuation.
pub fn describe_resources(
    p: &Program,
    w: &World,
    frame: &Frame,
    operands: [u8; 8],
    id: u64,
    host: &mut impl NeighborHost,
) -> Result<Description, String> {
    let resource_host = std::cell::RefCell::new(host);
    let (record, formatted_resource, named) =
        super::describe_using(p, w, frame, operands, id, true, &mut |text, id| {
            let mut selected = 0;
            let text = super::format_neighbor(
                text,
                id,
                None,
                Some(&mut selected),
                |id| {
                    let resource = resource_host.borrow_mut().neighbor(id)?;
                    if resource == Some(0) {
                        return Err("null dialog resource must be absent".into());
                    }
                    Ok(resource)
                },
                |resource| resource_host.borrow_mut().name(*resource),
            )?;
            Ok((text, (selected != 0).then_some(selected)))
        })?;
    let resource = match (operands[7] >> 1) & 7 {
        0 => match formatted_resource {
            Some(resource) => Some(resource),
            None => match record
                .icon_object
                .and_then(|id| w.objects.get(&id))
                .and_then(|o| o.definition.as_ref())
            {
                Some(definition) => resource_host.borrow_mut().live(definition)?,
                None => None,
            },
        },
        // Mode two discards the formatter's resource and resolves Stack Object
        // in the neighborhood namespace again, even after a successful lookup.
        2 if frame.stack_object > 0 => resource_host.borrow_mut().neighbor(frame.stack_object)?,
        4 => match named {
            Some(super::NamedIcon::Guid(guid)) => resource_host.borrow_mut().guid(guid)?,
            _ => None,
        },
        _ => None,
    };
    if resource == Some(0) {
        return Err("null dialog resource must be absent".into());
    }
    Ok(Description {
        record,
        resource,
        relationship: named == Some(super::NamedIcon::Relationship),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    id: u64,
    tick: u64,
    frame: Frame,
    operands: [u8; 8],
    returned: u8,
}
impl Construction {
    pub(super) fn new(id: u64, tick: u64, frame: Frame, operands: [u8; 8], returned: u8) -> Self {
        Self {
            id,
            tick,
            frame,
            operands,
            returned,
        }
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn named_icon(
        &self,
        program: &Program,
        world: &World,
    ) -> Result<Option<super::NamedIcon>, String> {
        if (self.operands[7] >> 1) & 7 != 4 {
            return Ok(None);
        }
        let (_, _, named) = super::describe_using(
            program,
            world,
            &self.frame,
            self.operands,
            self.id,
            false,
            &mut |_, _| Err("Dialog neighbor resource host absent".into()),
        )?;
        Ok(named)
    }

    pub fn relationship_icon(&self, program: &Program, world: &World) -> Result<bool, String> {
        Ok(self.named_icon(program, world)? == Some(super::NamedIcon::Relationship))
    }

    pub fn guid_icon(&self, program: &Program, world: &World) -> Result<Option<u32>, String> {
        Ok(match self.named_icon(program, world)? {
            Some(super::NamedIcon::Guid(guid)) => Some(guid),
            _ => None,
        })
    }

    /// Finish text passes and explicit resource selection before any portrait
    /// callback. The caller saves this result and does not format it again.
    pub fn describe(
        &self,
        p: &Program,
        w: &World,
        host: &mut impl NeighborHost,
    ) -> Result<Description, String> {
        describe_resources(p, w, &self.frame, self.operands, self.id, host)
    }

    pub(crate) fn validate(&self, vm: &Vm, p: &Program, frame: &Frame) -> Result<(), String> {
        // The executing tree may be semiglobal; its file is not the object's
        // code-resource context. Preserve the same owner used by VM calls.
        let (source, routine) = p.resolve(&self.frame.code_owner, self.frame.location.bhav)?;
        let ins = routine
            .body
            .instructions
            .get(self.frame.location.instruction)
            .ok_or("dialog construction instruction absent")?;
        if self.id == 0
            || source != self.frame.location.source
            || self.id >= vm.next_request
            || self.tick != vm.tick
            || self.frame.location != frame.location
            || self.frame.code_owner != frame.code_owner
            || self.frame.caller != frame.caller
            || self.frame.callee != frame.callee
            || self.frame.stack_object != frame.stack_object
            || self.frame.storage_offset != frame.storage_offset
            || self.frame.locals.len() != frame.locals.len()
            || self.frame.args.len() != frame.args.len()
            || ins.opcode != 0x24
            || ins.operands != self.operands
            || !super::presentation::supported(self.operands[5])
            || (!matches!((self.operands[7] >> 1) & 7, 0 | 2)
                && !matches!(
                    self.named_icon(p, &vm.world)?,
                    Some(super::NamedIcon::Relationship | super::NamedIcon::Guid(_))
                ))
            || self.returned != if self.operands[7] & 1 != 0 { 1 } else { 2 }
            || !vm.world.objects.contains_key(&self.frame.caller)
            || vm.world.dialogs.records.iter().any(|r| r.id == self.id)
        {
            return Err("invalid synchronous dialog construction".into());
        }
        Ok(())
    }
}

impl Vm {
    pub fn dialog_construction(&self, thread: crate::ThreadId) -> Result<&Construction, String> {
        match &self
            .threads
            .get(&thread)
            .ok_or("dialog thread absent")?
            .status
        {
            Status::AwaitingDialog(request) => Ok(request),
            _ => Err("thread is not constructing a dialog".into()),
        }
    }

    /// True also during a reentrant callback above the suspended instruction.
    pub fn constructing_dialog(&self) -> bool {
        self.threads.values().any(|t| {
            std::iter::once(&t.status)
                .chain(
                    t.callback
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| &b.resume_status),
                )
                .chain(
                    t.service
                        .iter()
                        .flat_map(|b| b.chain())
                        .map(|b| &b.resume_status),
                )
                .any(|s| matches!(s, Status::AwaitingDialog(_)))
        })
    }

    pub(crate) fn validate_dialog_image_ownership(
        &self,
        extra: Option<&super::Image>,
    ) -> Result<(), String> {
        let cache = &self.world.image_cache;
        let mut owners = vec![0u32; cache.refs.len()];
        for id in cache
            .entries
            .iter()
            .flat_map(|e| e.images)
            .flatten()
            .chain(
                self.world
                    .dialogs
                    .records
                    .iter()
                    .filter_map(|r| r.portrait.as_ref().map(|i| i.handle)),
            )
            .chain(extra.map(|i| i.handle))
        {
            let n = owners
                .get_mut(id.wrapping_sub(1) as usize)
                .ok_or("dialog image absent")?;
            *n += 1;
        }
        if owners
            .iter()
            .zip(&cache.refs)
            .any(|(owners, refs)| owners > refs)
        {
            return Err("dialog image ownership exceeds references".into());
        }
        Ok(())
    }

    /// Transfer the constructor's image wrapper, publish exactly once, then
    /// return from the original primitive. Callback changes are retained.
    pub fn finish_dialog_construction(
        &mut self,
        p: &Program,
        thread: crate::ThreadId,
        record: Record,
    ) -> Result<(), String> {
        let request = self.dialog_construction(thread)?.clone();
        let t = &self.threads[&thread];
        request.validate(self, p, t.stack.last().ok_or("dialog frame absent")?)?;
        if record.id != request.id
            || record.caller != request.frame.caller
            || record.origin != request.frame.location
            || record.source != request.frame.code_owner
            || record.stack_object != request.frame.stack_object
            || record.operands != request.operands
            || record.status != 0
            || (request.operands[7] >> 1) & 7 != 0 && record.icon_object.is_some()
            || record.job_icon.is_some()
            || record.bitmap_icon.is_some()
            || record.global_bitmap_icon.is_some()
            || record.guid_icon != request.guid_icon(p, &self.world)?
            || record.title.len() > 65536
            || record.body.len() > 65536
            || record.buttons.len() != super::presentation::button_count(request.operands[5])
            || record
                .buttons
                .iter()
                .enumerate()
                .any(|(i, b)| b.result != i as u32 + 2 || b.label.len() > 65536)
            || self.world.dialogs.records.len() >= 4096
            || record.portrait.as_ref().is_some_and(|image| {
                !super::portrait_rect(request.operands[7], record.guid_icon, image.rect)
            })
        {
            return Err("dialog construction result does not match request".into());
        }
        self.validate_dialog_image_ownership(record.portrait.as_ref())?;
        let (_, routine) = p.resolve(&request.frame.code_owner, request.frame.location.bhav)?;
        let yes = routine.body.instructions[request.frame.location.instruction].on_true;
        let wake = self.tick.checked_add(1).ok_or("Dialog tick overflow")?;
        self.world.ui_owner.dialog_owners.push(record.caller);
        self.world.dialogs.records.push(record);
        let mut t = self.threads.remove(&thread).unwrap();
        if request.returned == 2 {
            t.status = Status::Sleeping { wake_tick: wake };
        } else {
            t.status = Status::Ready;
            if let Err(e) = self.branch_in_thread(p, thread, &mut t, yes) {
                crate::fault(&mut t, e);
            }
        }
        if let Err(e) = self.finish_action_frames(&mut t) {
            crate::fault(&mut t, e);
        }
        if let Err(e) = self.continue_lifecycle(p, thread, &mut t) {
            crate::fault(&mut t, e);
        }
        self.threads.insert(thread, t);
        Ok(())
    }
}
