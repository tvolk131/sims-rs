//! Resumable residential Find Best Action. Original eligibility trees run on
//! the caller's stack; diagnostic budgets never become simulated yields.
use super::{Candidate, Enumeration, Selection, Tuning};
use crate::{ObjectId, Program, Thread, Vm, motives};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub object: ObjectId,
    pub menu: u32,
    pub distance: f32,
    pub attenuation: f32,
    /// Native action kind 1 supplies check argument 1 = 1, even though the
    /// selected object/menu are also reachable as an ordinary candidate.
    pub join: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Query {
    pub actor: ObjectId,
    pub parent_depth: usize,
    pub baseline: f32,
    pub threshold: f32,
    pub subset: u8,
    pub objects: Vec<ObjectId>,
    pub next_object: usize,
    pub pending: Vec<Pending>,
    pub next_pending: usize,
    pub waiting: Option<Pending>,
    pub saved_temps: Option<[i16; 8]>,
    pub candidates: Vec<Candidate>,
    pub tuning: Tuning,
    pub curves: Vec<motives::Curve>,
}
fn profile(vm: &Vm, actor: i16) -> Result<[i16; 256], String> {
    let fields = vm
        .world
        .objects
        .get(&actor)
        .and_then(|o| o.person.as_ref())
        .ok_or("autonomy person missing")?;
    let mut result = [0; 256];
    for i in [
        0, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 15, 17, 18, 32, 34, 36, 55, 58, 65,
    ] {
        result[i] = *fields
            .get(&(i as i16))
            .ok_or_else(|| format!("autonomy person field {i} missing"))?;
    }
    Ok(result)
}
impl Query {
    fn start(vm: &Vm, p: &Program, t: &Thread) -> Result<Self, String> {
        let actor = t.stack.last().unwrap().caller;
        let context = vm
            .world
            .person
            .as_ref()
            .and_then(|w| w.autonomy.as_ref())
            .ok_or("autonomy spatial context unavailable")?;
        if !context.residential {
            return Err("autonomy on nonresidential lots is unsupported".into());
        }
        if vm.world.tree_advertisements.len() != 16 {
            return Err("autonomy needs sixteen shared tree advertisements".into());
        }
        let person = profile(vm, actor)?;
        let tuning = Tuning::from_world(&vm.world)?;
        let curves = motives::score_owner::current(&vm.world, p, actor)?;
        let baseline = motives::score(
            vm.world.objects[&actor]
                .motives
                .as_ref()
                .ok_or("autonomy motives unavailable")?,
            &person,
            &curves,
            None,
        )?;
        Ok(Self {
            actor,
            parent_depth: t.stack.len(),
            baseline,
            threshold: if person[32] == 0 {
                tuning.family_threshold
            } else {
                tuning.visitor_threshold
            },
            subset: if match &context.full_scan {
                Some(set) => set.contains(&actor),
                None => {
                    let lot = *vm
                        .world
                        .globals
                        .get(&10)
                        .ok_or("autonomy current lot absent")?;
                    super::full_lot_scan(lot, &vm.world.person.as_ref().unwrap().lot_zoning)
                }
            } {
                0
            } else {
                (vm.tick.wrapping_add(actor as u16 as u64) & 3) as u8
            },
            objects: vm
                .world
                .objects
                .keys()
                .filter(|&&id| id > 0)
                .copied()
                .collect(),
            next_object: 0,
            pending: vec![],
            next_pending: 0,
            waiting: None,
            saved_temps: None,
            candidates: vec![],
            tuning,
            curves,
        })
    }
    fn next(&mut self, vm: &Vm, p: &Program) -> Result<Option<Pending>, String> {
        loop {
            if let Some(pending) = self.pending.get(self.next_pending).cloned() {
                self.next_pending += 1;
                return Ok(Some(pending));
            }
            self.pending.clear();
            self.next_pending = 0;
            let Some(&id) = self.objects.get(self.next_object) else {
                return Ok(None);
            };
            self.next_object += 1;
            let object = vm
                .world
                .objects
                .get(&id)
                .ok_or("autonomy target removed during scan")?;
            let person = profile(vm, self.actor)?;
            let context = vm.world.person.as_ref().unwrap().autonomy.as_ref().unwrap();
            if !context.advertised.as_ref().map_or_else(
                || object.idle.as_ref().is_some_and(|s| s.flags & 8 != 0),
                |set| set.contains(&id),
            ) || (id == self.actor && person[65] < 2)
            {
                continue;
            }
            let def = object
                .definition
                .as_ref()
                .ok_or("advertised target definition unavailable")?;
            let words = crate::construction::definition_words(p, def)?;
            if words[7] == 0
                || *object
                    .data
                    .get(&25)
                    .ok_or("autonomy target availability missing")?
                    >= 1
            {
                continue;
            }
            let kind = words[9];
            if (self.subset == 2 && id & 1 != 0) || (self.subset == 3 && id & 1 == 0) {
                continue;
            }
            let position = |id| {
                context
                    .placements
                    .get(&id)
                    .copied()
                    .ok_or("autonomy placement missing")
            };
            let distance = super::distance(position(self.actor)?, position(id)?, kind);
            if !distance.is_finite() {
                return Err("nonfinite native autonomy distance".into());
            }
            if self.subset == 1 && distance > 7. {
                continue;
            }
            // 0x458300 normalizes the interaction target only after distance
            // gating on the scanned part. Native enumeration may retain duplicates.
            let id = crate::multitile::interaction_target(&vm.world, p, id)?;
            let object = &vm.world.objects[&id];
            let def = object
                .definition
                .as_ref()
                .ok_or("redirected definition missing")?;
            let kind = crate::construction::definition_words(p, def)?[9];
            let enumeration = Enumeration {
                age: person[58],
                species: person[65],
                visitor: person[32],
                live_family: vm
                    .world
                    .actions
                    .as_ref()
                    .ok_or("autonomy action services missing")?
                    .live_family,
                level: person[36],
                pet_permission: person[34],
                target_kind: kind,
                target_room: *object.data.get(&29).ok_or("autonomy target room missing")?,
            };
            for entry in vm.action_menu(p, id)?.iter().rev() {
                if enumeration.permits(entry.flags, entry.autonomy.threshold as i16)? {
                    self.pending.push(Pending {
                        join: false,
                        object: id,
                        menu: entry.id,
                        distance: distance as f32,
                        attenuation: self.tuning.attenuation(
                            entry.autonomy.attenuation_code,
                            entry.autonomy.attenuation,
                            person[32] != 0,
                        ),
                    });
                }
            }
            if kind == 2
                && let Some((target, menu)) = vm.autonomy_join_target(p, id)?
            {
                let entry = vm
                    .action_menu(p, target)?
                    .iter()
                    .find(|e| e.id == menu)
                    .unwrap();
                self.pending.push(Pending {
                    join: true,
                    object: target,
                    menu,
                    // 0x45e0b1 measures the scanned person, even though the
                    // join constructor changes the candidate's object/menu.
                    distance: distance as f32,
                    attenuation: self.tuning.attenuation(
                        entry.autonomy.attenuation_code,
                        entry.autonomy.attenuation,
                        person[32] != 0,
                    ),
                });
            }
        }
    }
    fn score(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        pending: Pending,
        allowed: bool,
    ) -> Result<(), String> {
        let room = |id| {
            vm.world.objects[&id]
                .data
                .get(&29)
                .copied()
                .ok_or("autonomy room missing")
        };
        if room(self.actor)? != room(pending.object)? {
            let ad = &mut vm.world.tree_advertisements[13];
            ad[2] = ad[1].wrapping_neg();
        }
        if !allowed {
            return Ok(());
        }
        let person = profile(vm, self.actor)?;
        let ads: [motives::Advertisement; 16] = std::array::from_fn(|i| {
            let a = vm.world.tree_advertisements[i];
            motives::Advertisement {
                personality: a[0],
                minimum: a[1],
                delta: a[2],
            }
        });
        let prospective = motives::score_extended(
            vm.world.objects[&self.actor]
                .motives
                .as_ref()
                .ok_or("autonomy motives missing")?,
            &person,
            &self.curves,
            Some(&ads),
        )?;
        if prospective == 0. {
            return Ok(());
        }
        let factor =
            (1. / (f64::from(pending.attenuation) * f64::from(pending.distance) + 1.)) as f32;
        let improvement = (prospective - f64::from(self.baseline)) * f64::from(factor);
        if !improvement.is_finite() {
            return Err("nonfinite autonomy candidate score".into());
        }
        if improvement >= f64::from(self.threshold) {
            let entry = vm
                .action_menu(p, pending.object)?
                .iter()
                .find(|e| e.id == pending.menu)
                .ok_or("autonomy entry disappeared")?;
            self.candidates.push(Candidate {
                object: pending.object,
                menu: pending.menu,
                score: improvement as f32,
                force_best: entry.flags & 256 != 0,
            });
            if self.candidates.len() > 65536 {
                return Err("autonomy candidate diagnostic limit reached".into());
            }
        }
        Ok(())
    }
}
impl Vm {
    /// Autonomy's person tail (0x458b1c) and the target/menu portion of the join
    /// constructor (0x41ee99). A missing or invalid join record is unavailable;
    /// its threshold/age flags are not re-enumerated as an ordinary TTAB entry.
    pub fn autonomy_join_target(
        &self,
        p: &Program,
        person: ObjectId,
    ) -> Result<Option<(ObjectId, u32)>, String> {
        let actor = self
            .world
            .actions
            .as_ref()
            .and_then(|a| a.actors.get(&person))
            .ok_or("autonomy target person action storage unavailable")?;
        let Some(active) = &actor.active else {
            return Ok(None);
        };
        if active.target == 0 {
            return Ok(None);
        }
        let table = |target| -> Result<Option<&[crate::program::MenuEntry]>, String> {
            let object = self
                .world
                .objects
                .get(&target)
                .ok_or("join action target absent")?;
            let definition = object
                .definition
                .as_ref()
                .ok_or("join target definition absent")?;
            let module = p
                .modules
                .get(&definition.source)
                .ok_or("join target module absent")?;
            Ok(module
                .object_interactions
                .get(&definition.object)
                .map(Vec::as_slice))
        };
        let Some(entry) =
            table(active.target)?.and_then(|t| t.iter().find(|e| e.id == active.menu_id))
        else {
            return Ok(None);
        };
        if entry.flags & 2 == 0 {
            return Ok(None);
        }
        let target = crate::multitile::interaction_target(&self.world, p, active.target)?;
        let Some(entries) = table(target)? else {
            return Ok(None);
        };
        let Some(entry) = entries.iter().find(|e| e.id == active.menu_id) else {
            return Ok(None);
        };
        let join = entry.autonomy.joining_index;
        Ok((join != -1 && entries.iter().any(|e| e.id == join as u32))
            .then_some((target, join as u32)))
    }

    pub(crate) fn find_best_interaction(
        &mut self,
        p: &Program,
        t: &mut Thread,
        yes: u8,
        no: u8,
    ) -> Result<String, String> {
        self.find_best_interaction_with_observation(p, t, yes, no, &mut crate::action_observation::Observation::disabled())
    }

    pub(crate) fn find_best_interaction_with_observation(
        &mut self,
        p: &Program,
        t: &mut Thread,
        yes: u8,
        no: u8,
        observation: &mut crate::action_observation::Observation<'_>,
    ) -> Result<String, String> {
        if t.check || t.dispatch.is_some() {
            return Err("nested autonomy search inside a check/dispatch is unsupported".into());
        }
        if t.autonomy.is_none() {
            if self.autonomy_disabled(t)? {
                t.last_result = false;
                crate::branch_observed(p, t, no, Some(&self.world), observation)?;
                return Ok("native free-will gate declined autonomy".into());
            }
            let state = self.world.person.as_ref().unwrap();
            let context = state
                .autonomy
                .as_ref()
                .ok_or("autonomy spatial context unavailable")?;
            if context.suppressed
                || (state.controlled_family == 0
                    && context.selected_person == Some(t.stack.last().unwrap().caller))
            {
                t.last_result = false;
                crate::branch_observed(p, t, no, Some(&self.world), observation)?;
                return Ok("native lot control gate declined autonomy".into());
            }
            t.autonomy = Some(Query::start(self, p, t)?);
            t.callback_result = None;
        }
        let mut q = t.autonomy.take().unwrap();
        if let Some(pending) = q.waiting.take() {
            let allowed = t
                .callback_result
                .take()
                .ok_or("autonomy eligibility has not returned")?;
            q.score(self, p, pending, allowed)?;
            t.autonomy = Some(q);
            return Ok("Find Best Action scored eligibility callback".into());
        }
        if let Some(pending) = q.next(self, p)? {
            let entry = self
                .action_menu(p, pending.object)?
                .iter()
                .find(|e| e.id == pending.menu)
                .ok_or("autonomy menu missing")?
                .clone();
            let allowed = self.action_eligibility_gate(q.actor, pending.object, entry.flags)?
                && self.world.objects[&pending.object].data[&25] < 1;
            if allowed {
                for (dst, ad) in self
                    .world
                    .tree_advertisements
                    .iter_mut()
                    .zip(&entry.autonomy.motives)
                {
                    *dst = [ad.personality, ad.minimum, ad.delta];
                }
                if entry.check == 0 {
                    q.score(self, p, pending, true)?;
                } else {
                    q.saved_temps
                        .get_or_insert(self.world.objects[&q.actor].temps);
                    let owner = self.world.objects[&pending.object]
                        .definition
                        .as_ref()
                        .unwrap()
                        .source
                        .clone();
                    if !self.try_push_callback_start(
                        p,
                        t,
                        crate::Start {
                            owner: &owner,
                            bhav: entry.check,
                            caller: q.actor,
                            callee: pending.object,
                            stack_object: pending.object,
                            args: crate::action::eligibility_arguments(
                                true,
                                u32::from(pending.join),
                                0,
                                entry.check,
                            ),
                            check: true,
                        },
                    )? {
                        q.score(self, p, pending, false)?;
                        t.autonomy = Some(q);
                        return Ok("Find Best Action rejected absent eligibility tree".into());
                    }
                    q.waiting = Some(pending);
                }
            }
            t.autonomy = Some(q);
            return Ok("Find Best Action advanced native residential eligibility scan".into());
        }
        let person = profile(self, q.actor)?;
        let chosen = super::select(
            &mut q.candidates,
            &Selection {
                posture: person[0],
                person18: person[18],
                visitor: person[32],
                live_family: self.world.actions.as_ref().unwrap().live_family,
                random_count: q.tuning.random_count,
                sitting_threshold: q.tuning.sitting_threshold,
            },
            &mut self.rng_state,
        )?;
        if let Some(temps) = q.saved_temps {
            self.world.objects.get_mut(&q.actor).unwrap().temps = temps;
        }
        let result = chosen.is_some();
        if let Some(candidate) = chosen {
            self.world
                .objects
                .get_mut(&q.actor)
                .unwrap()
                .data
                .insert(20, candidate.menu as i16);
            t.stack.last_mut().unwrap().stack_object = candidate.object;
        }
        t.last_result = result;
        crate::branch_observed(p, t, if result { yes } else { no }, Some(&self.world), observation)?;
        Ok(format!(
            "Find Best Action evaluated {} candidates; selected={result}",
            q.candidates.len()
        ))
    }
}

impl Query {
    pub(crate) fn validate(&self, vm: &Vm, p: &Program, t: &Thread) -> Result<(), String> {
        let parent = t
            .stack
            .get(
                self.parent_depth
                    .checked_sub(1)
                    .ok_or("invalid autonomy depth")?,
            )
            .ok_or("missing autonomy parent")?;
        let (_, routine) = p.resolve(&parent.code_owner, parent.location.bhav)?;
        if parent.caller != self.actor
            || routine
                .body
                .instructions
                .get(parent.location.instruction)
                .is_none_or(|i| i.opcode != 3)
            || t.dispatch.is_some()
            || t.service.is_some()
            // Keep a diagnostic fault's query/callback binding inspectable and
            // saveable. It cannot execute until the unsupported service is fixed.
            || !matches!(t.status, crate::Status::Ready | crate::Status::Faulted(_))
            || !self.baseline.is_finite()
            || !self.threshold.is_finite()
            || self.subset > 3
            || self.objects.len() > 32767
            || self.objects.windows(2).any(|s| s[0] >= s[1])
            || self
                .objects
                .iter()
                .any(|i| *i <= 0 || !vm.world.objects.contains_key(i))
            || self.next_object > self.objects.len()
            || self.next_pending > self.pending.len()
            || self.pending.len() > 65536
            || self.candidates.len() > 65536
            || self.tuning != Tuning::from_world(&vm.world)?
        {
            return Err("invalid saved autonomy search".into());
        }
        let expected = motives::score_owner::current(&vm.world, p, self.actor)?;
        if motives::score_owner::Owner::capture(&self.curves)?
            != motives::score_owner::Owner::capture(&expected)? {
            return Err("saved autonomy curve binding mismatch".into());
        }
        for pending in self.pending.iter().chain(self.waiting.iter()) {
            if !pending.distance.is_finite()
                || !pending.attenuation.is_finite()
                || !vm
                    .action_menu(p, pending.object)?
                    .iter()
                    .any(|e| e.id == pending.menu)
            {
                return Err("invalid autonomy candidate binding".into());
            }
        }
        for c in &self.candidates {
            if !c.score.is_finite() || !vm.action_menu(p, c.object)?.iter().any(|e| e.id == c.menu)
            {
                return Err("invalid saved autonomy score".into());
            }
        }
        match (&self.waiting, &t.callback) {
            (Some(w), Some(b)) => {
                let entry = vm
                    .action_menu(p, w.object)?
                    .iter()
                    .find(|e| e.id == w.menu)
                    .unwrap();
                let child = t
                    .stack
                    .get(self.parent_depth)
                    .ok_or("missing autonomy callback frame")?;
                if self.saved_temps.is_none()
                    || b.base_depth != self.parent_depth
                    || b.resume_check
                    || b.resume_status != crate::Status::Ready
                    || !t.check
                    || child.caller != self.actor
                    || child.callee != w.object
                    || child.location.bhav != entry.check
                    || child.code_owner
                        != vm.world.objects[&w.object]
                            .definition
                            .as_ref()
                            .unwrap()
                            .source
                    || t.callback_result.is_some()
                {
                    return Err("invalid autonomy callback binding".into());
                }
            }
            (Some(_), None) => {
                if self.saved_temps.is_none()
                    || t.stack.len() != self.parent_depth
                    || t.callback_result.is_none()
                    || t.check
                {
                    return Err("invalid completed autonomy callback".into());
                }
            }
            (None, None) => {
                if t.stack.len() != self.parent_depth || t.callback_result.is_some() || t.check {
                    return Err("invalid autonomy scan continuation".into());
                }
            }
            _ => return Err("unbound autonomy callback".into()),
        }
        Ok(())
    }
}
