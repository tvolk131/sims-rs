//! Main installation and synchronous Init boundaries, with reusable frame storage.
use crate::{
    CallbackBoundary, Frame, Location, ObjectId, Program, Start, Status, Thread, ThreadId, Vm,
    dimensions, frame_words, save_frame_storage,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    Init,
    Main,
    Finished,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lifecycle {
    pub owner: String,
    pub definition: u16,
    pub init: u16,
    pub main: u16,
    pub phase: Phase,
    pub thread: ThreadId,
    pub run_init: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceCall {
    pub parent: Option<Box<ServiceCall>>,
    pub base_depth: usize,
    pub resume_status: Status,
    pub owner: String,
    pub bhav: u16,
    pub caller: ObjectId,
    pub callee: ObjectId,
    pub result: Option<bool>,
}
impl ServiceCall {
    pub fn chain(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |s| s.parent.as_deref())
    }
}
/// Native 0x4749b5 installs this frame even when the following tree is absent.
pub(crate) fn write_callback_sentinel(t: &mut Thread) -> Result<usize, String> {
    let sentinel = t.stack.last().map_or(0, |parent| {
        parent.storage_offset + frame_words(parent.args.len(), parent.locals.len())
    });
    let offset = sentinel + 8;
    crate::retained_frames::ensure_capacity(t, offset)?;
    save_frame_storage(t)?;
    t.frame_memory
        .resize(t.frame_memory.len().max(offset), Some(0));
    t.frame_memory[sentinel..offset].copy_from_slice(&[
        Some(-1),
        Some(0),
        Some(0),
        Some(0),
        Some(1),
        Some(0),
        Some(0),
        Some(0),
    ]);
    Ok(offset)
}
impl Vm {
    pub fn pop_service(&mut self, id: ThreadId) -> Result<ServiceCall, String> {
        let t = self.threads.get_mut(&id).ok_or("missing service thread")?;
        let mut s = t.service.take().ok_or("missing service boundary")?;
        if s.result.is_none() {
            t.service = Some(s);
            return Err("service still running".into());
        }
        t.service = s.parent.take().map(|s| *s);
        Ok(s)
    }
    /// Start an original Main on explicitly supplied person storage. This does
    /// not claim to execute the general person constructor or Init.
    pub fn start_supplied_person_main(
        &mut self,
        program: &Program,
        actor: ObjectId,
    ) -> Result<ThreadId, String> {
        if self.object_threads.contains_key(&actor)
            || self
                .world
                .objects
                .get(&actor)
                .is_none_or(|o| o.person.is_none())
        {
            return Err("person Main needs an unregistered person".into());
        }
        self.install_lifecycle(program, actor, false, None)
    }
    pub fn push_service_call(
        &mut self,
        program: &Program,
        id: ThreadId,
        start: Start<'_>,
    ) -> Result<(), String> {
        let old = self.threads.get(&id).ok_or("missing service thread")?;
        if old.service.as_ref().is_some_and(|s| s.result.is_some())
            || old.callback.is_some()
            || old.dispatch.is_some()
            || old.autonomy.is_some()
            || old.check
            || !matches!(
                old.status,
                Status::Ready | Status::Sleeping { .. } | Status::AwaitingRoute(_)
            )
        {
            return Err("service call requires an ordinary ready/sleeping/routing parent".into());
        }
        if old.stack.len() >= crate::MAX_STACK {
            return Err("service call stack limit".into());
        }
        let context = self.retained_entry_context(old, start.callee, start.owner)?;
        let (_, routine) = program.resolve(start.owner, start.bhav)?;
        let (argc, locals) = dimensions(routine)?;
        let parent = old.stack.last().ok_or("service parent frame absent")?;
        let offset = parent.storage_offset + frame_words(parent.args.len(), parent.locals.len());
        crate::retained_frames::ensure_capacity(old, offset + frame_words(argc, locals))?;
        let spawned = self.spawn(program, start)?;
        let mut child = self.threads.remove(&spawned).unwrap().stack.remove(0);
        self.next_thread = spawned;
        let t = self.threads.get_mut(&id).unwrap();
        save_frame_storage(t)?;
        let parent = t.stack.last().unwrap();
        child.storage_offset =
            parent.storage_offset + frame_words(parent.args.len(), parent.locals.len());
        t.frame_memory.resize(
            t.frame_memory
                .len()
                .max(child.storage_offset + frame_words(child.args.len(), child.locals.len())),
            Some(0),
        );
        for (i, value) in child.locals.iter_mut().enumerate() {
            match t.frame_memory[child.storage_offset + 8 + child.args.len() + i] {
                Some(v) => *value = v,
                None => {
                    child.opaque_locals.insert(i);
                }
            }
        }
        t.service = Some(ServiceCall {
            parent: t.service.take().map(Box::new),
            base_depth: t.stack.len(),
            resume_status: t.status.clone(),
            owner: child.code_owner.clone(),
            bhav: child.location.bhav,
            caller: child.caller,
            callee: child.callee,
            result: None,
        });
        crate::retained_frames::finish_entry(t, child, context)?;
        t.status = Status::Ready;
        Ok(())
    }
    /// Portal crossing uses the person's virtual target-tree entry (0x45e59b),
    /// including temporary arguments, wakeup, reusable locals and target use count.
    pub fn push_portal_service(
        &mut self,
        p: &Program,
        id: ThreadId,
        owner: String,
        bhav: u16,
        target: ObjectId,
    ) -> Result<(), String> {
        self.push_route_service(p, id, owner, bhav, target, None)
    }
    /// Original target-tree entry with an explicit native argument buffer, used
    /// by routing failure feedback. Null still means the caller's temporaries.
    pub fn push_route_service(
        &mut self,
        p: &Program,
        id: ThreadId,
        owner: String,
        bhav: u16,
        target: ObjectId,
        args: Option<&[i16]>,
    ) -> Result<(), String> {
        self.push_route_service_state(p, id, owner, bhav, target, args, None)
    }
    /// Original 0x45c7c2 portal entry publishes the parent's state 2/return 3
    /// at 0x45b954..0x45b9c0. Other route services retain their continuation.
    pub fn push_native_portal_service(
        &mut self,
        p: &Program,
        id: ThreadId,
        owner: String,
        bhav: u16,
        target: ObjectId,
    ) -> Result<(), String> {
        let t = self.threads.get(&id).ok_or("missing native portal thread")?;
        let frame = t.stack.last().ok_or("native portal parent missing")?;
        if !matches!(super::current(p, t)?.opcode, 27 | 45)
            || !matches!(&t.status, Status::AwaitingRoute(r) if r.origin == frame.location)
        {
            return Err("native portal service needs its retained route parent".into());
        }
        self.push_route_service_state(p, id, owner, bhav, target, None, Some(2))
    }
    fn push_route_service_state(
        &mut self,
        p: &Program,
        id: ThreadId,
        owner: String,
        bhav: u16,
        target: ObjectId,
        args: Option<&[i16]>,
        parent_state: Option<u32>,
    ) -> Result<(), String> {
        let before = self.clone();
        let mut t = self.threads.remove(&id).ok_or("missing portal thread")?;
        let result = (|| {
            if t.check
                || t.callback.is_some()
                || !matches!(t.status, Status::AwaitingRoute(_))
                || t.service.as_ref().is_some_and(|s| s.result.is_some())
            {
                return Err("invalid portal entry boundary".into());
            }
            let base_depth = t.stack.len();
            let parent = t.stack.last().ok_or("portal parent missing")?;
            let caller = parent.caller;
            let continuation = parent_state.unwrap_or(parent.continuation);
            let resume_status = t.status.clone();
            self.push_target_tree_args(p, &mut t, owner.clone(), bhav, target, continuation, args)?;
            t.service = Some(ServiceCall {
                parent: t.service.take().map(Box::new),
                base_depth,
                resume_status,
                owner,
                bhav,
                caller,
                callee: target,
                result: None,
            });
            t.status = Status::Ready;
            Ok(())
        })();
        self.threads.insert(id, t);
        if result.is_err() {
            *self = before;
        }
        result
    }
    /// Install callbacks on host-supplied storage. Use reset_object_lifecycle for
    /// the original base-object reset contract as well.
    pub fn start_object_lifecycle(
        &mut self,
        program: &Program,
        object: ObjectId,
    ) -> Result<ThreadId, String> {
        if self.lifecycles.contains_key(&object) || self.object_threads.contains_key(&object) {
            return Err("object lifecycle already started".into());
        }
        self.install_lifecycle(program, object, true, None)
    }
    pub fn reset_object_lifecycle(
        &mut self,
        program: &Program,
        object: ObjectId,
        now: crate::construction::ObjectDate,
    ) -> Result<ThreadId, String> {
        if self
            .world
            .objects
            .get(&object)
            .is_some_and(|o| o.multitile.is_some())
        {
            return Err("multi-tile reset requires the ordered group reset service".into());
        }
        self.reset_base_object_lifecycle(program, object, now, false)
    }

    /// Base431be1 inside cursor423175. This installs its own Init when the
    /// cursor's OBJD group is zero. The saved subclass owner must finish that
    /// callback, optional Update and registry flags before resetting the next
    /// member, and later execute the outer Init pass as well.
    pub fn begin_cursor_base_reset(
        &mut self,
        program: &Program,
        object: ObjectId,
        now: crate::construction::ObjectDate,
    ) -> Result<ThreadId, String> {
        let o = self
            .world
            .objects
            .get(&object)
            .ok_or("cursor reset object absent")?;
        if o.cursor.as_ref().is_none_or(|c| !c.postconstructed)
            || o.multitile.is_none()
            || o.container.is_some()
            || self
                .world
                .positions
                .get(&object)
                .is_none_or(|p| p.x != -16 || p.y != -16 || p.level != 1)
        {
            return Err("cursor base reset requires constructed off-lot storage".into());
        }
        self.reset_base_object_lifecycle(program, object, now, false)
    }

    /// Base Reset inside a newly registered, off-lot person's 0x4598e8 wrapper.
    /// Installs Main and starts the first Init. The caller must finish Init,
    /// the immediate person update with native argument 0, and the outer reset
    /// (including its second Init) before publishing the completed factory.
    pub fn begin_person_base_reset(
        &mut self,
        program: &Program,
        object: ObjectId,
        now: crate::construction::ObjectDate,
    ) -> Result<ThreadId, String> {
        let o = self
            .world
            .objects
            .get(&object)
            .ok_or("person constructor object missing")?;
        let definition = o
            .definition
            .as_ref()
            .ok_or("person constructor definition missing")?;
        let words = crate::construction::definition_words(program, definition)?;
        if o.person.is_none()
            || words[9] != 2
            || words[10] != 0
            || o.multitile.is_some()
            || o.container.is_some()
            || self
                .world
                .positions
                .get(&object)
                .is_none_or(|p| p.x != -16 || p.y != -16 || p.level != 1)
            || self.object_threads.contains_key(&object)
        {
            return Err(
                "person constructor base reset requires fresh off-lot single-part storage".into(),
            );
        }
        self.reset_base_object_lifecycle(program, object, now, true)
    }
    /// First pass of cXMTObject::Reset: all base resets precede any Init.
    /// The host then calls begin_multitile_init and completes one immediate
    /// update for each returned part in order. No lot time advances in between.
    pub fn prepare_multitile_reset(
        &mut self,
        program: &Program,
        member: ObjectId,
        now: crate::construction::ObjectDate,
    ) -> Result<Vec<ObjectId>, String> {
        let ids = crate::multitile::chain(&self.world, member)?;
        if ids.iter().any(|id| self.world.objects[id].cursor.is_some()) {
            //The cursor's OBJD group word is zero: each 431be1 therefore runs
            //its own Init/update/flags before the next part's reset. 423175
            //then calls Init again. The cursor reset owner must drive these
            //synchronous boundaries; batching their base resets changes order.
            return Err("cursor reset requires its ordered subclass service".into());
        }
        let before = self.clone();
        let result = (|| {
            for &id in &ids {
                self.reset_base_object_lifecycle(program, id, now, false)?;
            }
            crate::multitile::update_offsets(&mut self.world, member)?;
            Ok(ids)
        })();
        if result.is_err() {
            *self = before;
        }
        result
    }
    pub fn begin_multitile_init(
        &mut self,
        program: &Program,
        object: ObjectId,
    ) -> Result<(), String> {
        if self
            .world
            .objects
            .get(&object)
            .is_none_or(|o| o.multitile.is_none())
        {
            return Err("multi-tile Init requires a linked part".into());
        }
        let l = self
            .lifecycles
            .get(&object)
            .ok_or("missing group lifecycle")?
            .clone();
        if l.run_init || l.phase != Phase::Main || self.threads[&l.thread].status != Status::Ready {
            return Err("multi-tile Init needs its fresh base reset".into());
        }
        let mut t = self.threads[&l.thread].clone();
        if l.init != 0 {
            self.push_callback(program, &mut t, object, &l.owner, l.init)?;
        }
        self.threads.insert(l.thread, t);
        let l = self.lifecycles.get_mut(&object).unwrap();
        l.run_init = true;
        l.phase = if l.init != 0 {
            Phase::Init
        } else {
            Phase::Main
        };
        Ok(())
    }
    fn reset_base_object_lifecycle(
        &mut self,
        program: &Program,
        object: ObjectId,
        now: crate::construction::ObjectDate,
        person_constructor: bool,
    ) -> Result<ThreadId, String> {
        self.native_replacement_preflight(
            program,
            object,
            self.object_threads.get(&object).copied(),
        )?;
        let previous = self
            .world
            .objects
            .get(&object)
            .ok_or("missing reset object")?
            .clone();
        let existing = self.object_threads.get(&object).copied();
        if existing.is_some_and(|id| self.lifecycles.get(&object).is_none_or(|l| l.thread != id))
            || self.threads.iter().any(|(id, t)| {
                Some(*id) != existing
                    && t.stack
                        .iter()
                        .any(|f| f.caller == object || f.callee == object)
            })
        {
            return Err("reset with external active executions is unsupported".into());
        }
        let mut reset = previous.clone();
        let run_init = if person_constructor {
            reset.reset_base_storage(program, object, now)?
        } else {
            reset.reset_storage(program, object, now)?
        };
        self.world.objects.insert(object, reset);
        match self.install_lifecycle(program, object, run_init, existing) {
            Ok(id) => Ok(id),
            Err(e) => {
                self.world.objects.insert(object, previous);
                Err(e)
            }
        }
    }
    pub(crate) fn install_lifecycle(
        &mut self,
        program: &Program,
        object: ObjectId,
        run_init: bool,
        existing: Option<ThreadId>,
    ) -> Result<ThreadId, String> {
        self.native_replacement_preflight(program, object, existing)?;
        let definition = self
            .world
            .objects
            .get(&object)
            .and_then(|o| o.definition.as_ref())
            .ok_or("object definition required for lifecycle")?
            .clone();
        let entries = program
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_functions.get(&definition.object))
            .ok_or("object callback table unavailable")?;
        let init = entries.first().map_or(0, |e| e.action);
        let main = entries.get(1).map_or(0, |e| e.action);
        if main == 0 {
            return Err("lifecycle without Main is not yet supported".into());
        }
        let main_dims = dimensions(program.resolve(&definition.source, main)?.1)?;
        if main_dims.0 > 4 {
            return Err("native Main arguments beyond four zero words are unsupported".into());
        }
        if run_init && init != 0 {
            let (argc, _) = dimensions(program.resolve(&definition.source, init)?.1)?;
            if argc > 8 {
                return Err("Init arguments exceed native temporary buffer".into());
            }
        }
        // Native reset clears the frame pointer vector, not its backing arena.
        let mut retained = None;
        let mut root_context = None;
        let mut memory = if let Some(id) = existing {
            let mut old = self.threads[&id].clone();
            root_context = self.retained_entry_context(&old, object, &definition.source)?;
            let root_end = frame_words(main_dims.0, main_dims.1);
            let end = if run_init && init != 0 {
                let (_, routine) = program.resolve(&definition.source, init)?;
                let (argc, locals) = dimensions(routine)?;
                root_end + 8 + frame_words(argc, locals)
            } else {
                root_end
            };
            crate::retained_frames::ensure_capacity(&old, end)?;
            save_frame_storage(&mut old)?;
            retained = old.retained_frame_arena;
            old.frame_memory
        } else {
            Vec::new()
        };
        let opaque_locals = (0..main_dims.1)
            .filter(|i| memory.get(8 + main_dims.0 + i) == Some(&None))
            .collect();
        let locals = (0..main_dims.1)
            .map(|i| {
                memory
                    .get(8 + main_dims.0 + i)
                    .copied()
                    .flatten()
                    .unwrap_or(0)
            })
            .collect();
        let id = self.spawn(
            program,
            Start {
                owner: &definition.source,
                bhav: main,
                caller: object,
                callee: object,
                stack_object: 0,
                args: vec![0; 4],
                check: false,
            },
        )?;
        let mut t = self.threads.remove(&id).unwrap();
        memory.resize(memory.len().max(t.frame_memory.len()), Some(0));
        t.frame_memory = memory;
        t.stack[0].locals = locals;
        t.stack[0].opaque_locals = opaque_locals;
        t.retained_frame_arena = retained;
        if t.retained_frame_arena.is_some() {
            crate::retained_frames::reset_contexts(&mut t);
            let root = t.stack.remove(0);
            crate::retained_frames::finish_entry(&mut t, root, root_context)?;
        }
        if run_init && init != 0 {
            // Prevalidated dimensions; newly allocated frame storage is deterministic
            // zero fill, while reused native words (including the sentinel) survive.
            if let Err(e) = self.push_callback(program, &mut t, object, &definition.source, init) {
                self.next_thread = id;
                return Err(e);
            }
        }
        let id = if let Some(previous_id) = existing {
            self.next_thread = id;
            previous_id
        } else {
            id
        };
        self.threads.insert(id, t);
        self.lifecycles.insert(
            object,
            Lifecycle {
                owner: definition.source,
                definition: definition.object,
                init,
                main,
                phase: if run_init && init != 0 {
                    Phase::Init
                } else {
                    Phase::Main
                },
                thread: id,
                run_init,
            },
        );
        self.world
            .objects
            .get_mut(&object)
            .unwrap()
            .idle
            .get_or_insert_with(Default::default);
        self.object_threads.insert(object, id);
        Ok(id)
    }
    fn push_callback(
        &self,
        program: &Program,
        t: &mut Thread,
        object: ObjectId,
        owner: &str,
        bhav: u16,
    ) -> Result<(), String> {
        self.push_callback_start(
            program,
            t,
            Start {
                owner,
                bhav,
                caller: object,
                callee: object,
                stack_object: 0,
                args: self.world.objects[&object].temps.to_vec(),
                check: true,
            },
        )
    }
    /// The synchronous wrapper returns false when BHAV lookup finds no tree,
    /// but still installs/removes its sentinel in the reusable frame arena.
    /// Missing linked modules remain errors from lookup_routine.
    pub(crate) fn try_push_callback_start(
        &self,
        program: &Program,
        t: &mut Thread,
        start: Start<'_>,
    ) -> Result<bool, String> {
        if program.lookup_routine(start.owner, start.bhav)?.is_none() {
            write_callback_sentinel(t)?;
            return Ok(false);
        }
        self.push_callback_start(program, t, start)?;
        Ok(true)
    }
    pub(crate) fn push_callback_start(
        &self,
        program: &Program,
        t: &mut Thread,
        start: Start<'_>,
    ) -> Result<(), String> {
        if t.stack.len() >= crate::MAX_STACK {
            return Err("callback stack limit".into());
        }
        let owner = start.owner;
        let bhav = start.bhav;
        let (source, routine) = program.resolve(owner, bhav)?;
        let (argc, locals) = dimensions(routine)?;
        let args = start
            .args
            .get(..argc)
            .ok_or("callback argument read exceeds supplied buffer")?
            .to_vec();
        let context = self.retained_entry_context(t, start.callee, owner)?;
        let offset = t.stack.last().map_or(8, |parent| {
            parent.storage_offset + frame_words(parent.args.len(), parent.locals.len()) + 8
        });
        crate::retained_frames::ensure_capacity(t, offset + frame_words(argc, locals))?;
        // Retained entry must reject unsupported input before writing a sentinel.
        let retained_pc = if t.retained_frame_arena.is_some() {
            Some(super::entry_pc(
                routine,
                start.args.first().copied().unwrap_or(0),
            )?)
        } else {
            None
        };
        let offset = write_callback_sentinel(t)?;
        t.frame_memory.resize(
            t.frame_memory.len().max(offset + frame_words(argc, locals)),
            Some(0),
        );
        let opaque_locals = (0..locals)
            .filter(|i| t.frame_memory[offset + 8 + argc + i].is_none())
            .collect();
        let locals = (0..locals)
            .map(|i| t.frame_memory[offset + 8 + argc + i].unwrap_or(0))
            .collect();
        t.callback = Some(CallbackBoundary {
            external: false,
            parent: t.callback.take().map(Box::new),
            base_depth: t.stack.len(),
            resume_status: t.status.clone(),
            resume_check: t.check,
        });
        t.callback_result = None;
        t.check = true;
        crate::retained_frames::finish_entry(
            t,
            Frame {
                continuation: 0,
                action_target: None,
                action_icon: false,
                code_owner: owner.into(),
                location: Location {
                    source: source.into(),
                    bhav,
                    instruction: match retained_pc {
                        Some(pc) => pc,
                        None => super::entry_pc(routine, start.args.first().copied().unwrap_or(0))?,
                    },
                },
                caller: start.caller,
                callee: start.callee,
                stack_object: start.stack_object,
                args,
                locals,
                opaque_locals,
                storage_offset: offset,
            },
            context,
        )?;
        t.status = Status::Ready;
        Ok(())
    }
    pub(crate) fn continue_lifecycle(
        &mut self,
        _program: &Program,
        id: ThreadId,
        t: &mut Thread,
    ) -> Result<(), String> {
        let Some(l) = self.lifecycles.values_mut().find(|l| l.thread == id) else {
            return Ok(());
        };
        if l.phase == Phase::Init && t.callback.is_none() {
            l.phase = Phase::Main;
        }
        if matches!(t.status, Status::Returned(_)) {
            l.phase = Phase::Finished;
        }
        Ok(())
    }
}
