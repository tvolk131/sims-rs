//! Join retained route frames to explicitly assigned unpublished request IDs.
//! No frame/route is started or normalized and no VM counter is allocated.
//! These are pending service/status owners, not complete ThreadControls: the
//! caller still owes registry/lifecycle, other controls, source admission and
//! the Simulation's retained routing/animation execution owners.
use super::{
    completed_invocations,
    completed_routes::{Person, Suspension},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::{AnimationRequest, ObjectId, RouteRequest, Status, lifecycle::ServiceCall, routing};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub actor: ObjectId,
    pub frame_pointer: u32,
    /// Replacement host identity, not a native allocator serial or timestamp.
    pub request: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub leaf: Status,
    pub service: Option<ServiceCall>,
    pub routes: BTreeMap<usize, RouteRequest>,
}

pub fn import(
    invocations: &completed_invocations::Imported,
    people: &BTreeMap<ObjectId, Person>,
    delivery: &[Delivery],
) -> Result<BTreeMap<ObjectId, Pending>> {
    ensure!(
        delivery.len() <= 1024,
        "retained request delivery extent exceeds bound"
    );
    let mut ids = BTreeSet::new();
    let mut supplied = BTreeMap::new();
    for d in delivery {
        ensure!(
            d.actor > 0
                && d.frame_pointer != 0
                && d.request != 0
                && d.request != u64::MAX
                && ids.insert(d.request)
                && supplied
                    .insert((d.actor, d.frame_pointer), d.request)
                    .is_none(),
            "duplicate/invalid retained request delivery"
        );
    }
    let mut consumed = BTreeSet::new();
    let mut result = BTreeMap::new();
    for (&actor, person) in people {
        ensure!(
            person.readset_sha256 == invocations.readset_sha256,
            "retained request stack/route source epoch differs"
        );
        let object = invocations
            .objects
            .get(&actor)
            .context("retained route actor stack missing")?;
        let frames = object
            .stack
            .as_ref()
            .context("retained route actor has no active stack")?
            .frames();
        let request = |pointer: u32, used: &mut BTreeSet<(ObjectId, u32)>| -> Result<u64> {
            let key = (actor, pointer);
            ensure!(
                used.insert(key),
                "source frame owns multiple retained deliveries"
            );
            supplied
                .get(&key)
                .copied()
                .context("retained source frame has no supplied delivery owner")
        };
        let mut routes = BTreeMap::new();
        for context in &person.contexts {
            let frame = frames
                .get(
                    context
                        .frame_depth
                        .checked_sub(1)
                        .context("zero retained route depth")?,
                )
                .context("retained route frame missing")?;
            ensure!(
                object.frame_pointers.get(context.frame_depth - 1) == Some(&context.frame_pointer),
                "retained route frame storage owner differs"
            );
            let (_, routine) = invocations
                .program
                .resolve(&frame.code_owner, frame.location.bhav)
                .map_err(anyhow::Error::msg)?;
            let ins = routine
                .body
                .instructions
                .get(frame.location.instruction)
                .context("retained route instruction missing")?;
            ensure!(
                ins.opcode == 27,
                "retained non-relative route requires another descriptor profile"
            );
            let routing::Preparation::Start(descriptor) =
                routing::relative_position(&ins.operands, 0)
            else {
                anyhow::bail!("retained relative route has invalid facing");
            };
            ensure!(
                descriptor.words() == context.descriptor.0 && frame.stack_object == context.target,
                "retained route descriptor differs from native instruction/context"
            );
            let request = RouteRequest {
                id: request(context.frame_pointer, &mut consumed)?,
                origin: frame.location.clone(),
                caller: frame.caller,
                target: context.target,
                no_failure_trees: descriptor.flags & 0x4000 == 0,
                destination: routing::Destination::Relative(descriptor),
            };
            ensure!(
                routes.insert(context.frame_depth, request).is_none(),
                "duplicate retained route frame owner"
            );
        }
        let mut service = None;
        for context in &person.contexts {
            let Suspension::Child {
                depth,
                callee,
                tree,
            } = context.suspension
            else {
                continue;
            };
            ensure!(
                depth == context.frame_depth + 1,
                "retained portal child boundary differs"
            );
            let parent = &frames[context.frame_depth - 1];
            let child = frames
                .get(depth - 1)
                .context("retained portal child frame missing")?;
            let binding = &invocations
                .objects
                .get(&callee)
                .context("retained portal definition missing")?
                .binding;
            let kind = invocations
                .program
                .modules
                .get(&binding.definition.source)
                .and_then(|m| m.object_definition_words.get(&binding.definition.object))
                .and_then(|w| w.get(9))
                .copied();
            ensure!(
                parent.continuation == 2
                    && child.action_target == Some(callee)
                    && child.callee == callee
                    && child.location.bhav == tree
                    && kind == Some(8),
                "retained state2 portal/use/definition ownership differs"
            );
            service = Some(ServiceCall {
                parent: service.map(Box::new),
                base_depth: context.frame_depth,
                resume_status: Status::AwaitingRoute(routes[&context.frame_depth].clone()),
                owner: child.code_owner.clone(),
                bhav: tree,
                caller: child.caller,
                callee,
                result: None,
            });
        }
        let leaf_frame = frames.last().context("retained actor leaf missing")?;
        let leaf = if let Some(route) = routes.get(&frames.len()) {
            ensure!(
                leaf_frame.continuation == 3
                    && person
                        .contexts
                        .last()
                        .is_some_and(|c| c.suspension == Suspension::Walking),
                "retained route leaf has no walking owner"
            );
            Status::AwaitingRoute(route.clone())
        } else {
            let (_, routine) = invocations
                .program
                .resolve(&leaf_frame.code_owner, leaf_frame.location.bhav)
                .map_err(anyhow::Error::msg)?;
            let ins = routine
                .body
                .instructions
                .get(leaf_frame.location.instruction)
                .context("retained animation instruction missing")?;
            ensure!(
                ins.opcode == 44 && leaf_frame.continuation == 0,
                "retained non-route leaf requires another service profile"
            );
            let pointer = *object
                .frame_pointers
                .last()
                .context("retained animation frame pointer missing")?;
            Status::AwaitingAnimation(AnimationRequest {
                id: request(pointer, &mut consumed)?,
                origin: leaf_frame.location.clone(),
                caller: leaf_frame.caller,
                operands: ins.operands,
            })
        };
        result.insert(
            actor,
            Pending {
                leaf,
                service,
                routes,
            },
        );
    }
    ensure!(
        consumed.len() == supplied.len(),
        "unclaimed retained request delivery"
    );
    Ok(result)
}
